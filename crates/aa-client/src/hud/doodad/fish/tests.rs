use super::*;

use std::collections::HashSet;

use aa_net::Event;
use aa_net::model::{Item, ItemDetail};
use aa_net::state::{Doodads, Units};

use crate::hud::api::{HudBridge, Shared};
use crate::hud::doodad::{Focus, Views, execute, world_doodad};
use crate::hud::dynamic_actions::{Shown, Source};

const BUY_SKILL: u32 = 21_904;
const CONVERT_SKILL: u32 = 22_201;
const FISH_STAND_GROUP: u32 = 17_262;
const FISH_TEMPLATE: u32 = 27_457;
/// `fish_details.max_weight` of 27457 is 561; grade 0 refund multiplier is 100.
const WEIGHT: f32 = 500.0;
const LENGTH: f32 = 280.0;
const REFUND: i64 = 133_689;

fn game() -> Option<GameData> {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    std::path::Path::new(&path)
        .exists()
        .then(|| GameData::load(&path).ok())
        .flatten()
}

fn inventory(template: Option<u32>) -> Inventory {
    let mut inventory = Inventory::default();
    inventory.equipment.slots[slot::BACKPACK] = template.map(|template_id| Item {
        template_id,
        count: 1,
        ..Default::default()
    });
    inventory
}

/// A fish with its detail tag 6 payload (`f32` weight, `f32` length).
fn fish_inventory(template: u32, weight: f32, length: f32) -> Inventory {
    let mut inventory = inventory(Some(template));
    let item = inventory.equipment.slots[slot::BACKPACK].as_mut().unwrap();
    item.detail = ItemDetail::Other(6);
    let mut payload = vec![0u8; 16];
    payload[..4].copy_from_slice(&weight.to_le_bytes());
    payload[4..8].copy_from_slice(&length.to_le_bytes());
    item.detail_payload = Some(payload);
    inventory
}

fn items(game: &GameData, kind: FishKind, func: u32) -> Option<&HashSet<u32>> {
    game.doodad.fish_items(kind, func)
}

/// `FUN_397cf5a0` / `FUN_397cf660`: the errors, the price/fish_details gates and the weighted refund.
#[test]
fn the_fish_stand_checks_match_the_native_errors() {
    let Some(mut game) = game() else { return };
    assert_eq!(
        game.doodad.fish_func(FISH_STAND_GROUP, BUY_SKILL),
        Some(FishFunc {
            kind: FishKind::Buy,
            func_id: 1
        })
    );
    assert_eq!(
        game.doodad.fish_func(FISH_STAND_GROUP, CONVERT_SKILL),
        Some(FishFunc {
            kind: FishKind::Convert,
            func_id: 2
        })
    );
    let buy = FishFunc {
        kind: FishKind::Buy,
        func_id: 1,
    };
    let convert = FishFunc {
        kind: FishKind::Convert,
        func_id: 1,
    };
    let buy_items = items(&game, buy.kind, buy.func_id).cloned();
    let convert_items = items(&game, convert.kind, convert.func_id).cloned();
    assert_eq!(
        check(&game, buy_items.as_ref(), &inventory(None), buy.kind),
        Err(361)
    );
    assert_eq!(
        check(
            &game,
            buy_items.as_ref(),
            &inventory(Some(23_605)),
            buy.kind
        ),
        Err(225),
        "the buy list is checked before the item kind (FUN_397cf5a0)"
    );
    assert_eq!(
        check(
            &game,
            buy_items.as_ref(),
            &inventory(Some(28_768)),
            buy.kind
        ),
        Err(225),
        "a type-6 backpack the stand does not handle"
    );
    assert_eq!(
        check(
            &game,
            buy_items.as_ref(),
            &inventory(Some(29_722)),
            buy.kind
        ),
        Err(88),
        "items.price 0 is not sellable"
    );
    assert_eq!(
        check(
            &game,
            buy_items.as_ref(),
            &inventory(Some(23_605)),
            buy.kind
        ),
        Err(225)
    );
    let sale = check(
        &game,
        buy_items.as_ref(),
        &fish_inventory(FISH_TEMPLATE, WEIGHT, LENGTH),
        buy.kind,
    )
    .unwrap();
    assert_eq!(
        (sale.template, sale.weight, sale.length, sale.refund),
        (FISH_TEMPLATE, Some(WEIGHT), Some(LENGTH), Some(REFUND))
    );
    // convert: a buy-only fish is not in the convert list; kind 6 is checked first
    assert_eq!(
        check(
            &game,
            convert_items.as_ref(),
            &inventory(Some(27_602)),
            convert.kind
        ),
        Err(225)
    );
    assert_eq!(
        check(
            &game,
            convert_items.as_ref(),
            &inventory(Some(23_605)),
            convert.kind
        ),
        Err(88)
    );
    assert_eq!(
        check(
            &game,
            convert_items.as_ref(),
            &fish_inventory(FISH_TEMPLATE, WEIGHT, LENGTH),
            convert.kind
        )
        .unwrap()
        .refund,
        Some(REFUND),
        "convert's itemInfo carries the same refund"
    );
    // no fish_details row: the buy check refuses it
    game.doodad.fish_max_weights.remove(&FISH_TEMPLATE);
    assert_eq!(
        check(
            &game,
            buy_items.as_ref(),
            &fish_inventory(FISH_TEMPLATE, WEIGHT, LENGTH),
            buy.kind
        ),
        Err(88)
    );
}

fn app(game: GameData, inventory: Inventory, skill: u32) -> App {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).expect("the UI runtime");
    let mut views = Views::default();
    views.fixture_doodad(2075, Entity::PLACEHOLDER);
    let generation = views.doodad_generation(2075).unwrap();
    let bridge = Shared::default();
    {
        let mut d = bridge.borrow_mut();
        assert!(d.game.0.set(game).is_ok());
        d.snapshot.labor = 100;
        d.pos = Vec3::new(10.0, 10.0, 100.0);
        d.doodad.focus = Some(Focus {
            obj_id: 2075,
            skills: vec![skill],
            key: -3.0,
            dist: 3.0,
        });
        d.dynamic_actions.replace(Some(Shown {
            source: Source::Doodad {
                obj_id: 2075,
                phase: FISH_STAND_GROUP,
                generation,
            },
            skills: vec![skill],
        }));
        d.dynamic_actions.request(1);
    }
    crate::hud::api::install(&mut rt, &bridge).expect("the HUD API");
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    rt.lua
        .load("FISH_INFO=nil; local original=X2DialogManager.RequestTaskDialog; X2DialogManager.RequestTaskDialog=function(self,task,info,owner) FISH_INFO=info; return original(self,task,info,owner) end")
        .exec()
        .expect("capture the dialog info");
    let mut doodads = Doodads::default();
    assert!(doodads.apply(&Event::DoodadsSpawned(vec![world_doodad(
        2075,
        6507,
        FISH_STAND_GROUP,
        Vec3::new(10.0, 10.0, 100.0)
    )])));
    let mut app = App::new();
    app.insert_non_send(HudBridge(bridge))
        .insert_non_send(rt)
        .insert_resource(doodads)
        .insert_resource(Units::default())
        .insert_resource(inventory)
        .insert_resource(views)
        .add_message::<NetCommand>()
        .add_message::<crate::hud::UseFailed>()
        .add_message::<crate::hud::ShowError>()
        .init_resource::<FishDialog>()
        .add_systems(Update, (execute, tick).chain());
    app
}

fn sent(app: &mut App) -> Vec<Command> {
    app.world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|command| command.0)
        .collect()
}

fn lua(app: &App, script: &str) -> mlua::Result<mlua::Value> {
    app.world()
        .get_non_send::<UiRuntime>()
        .unwrap()
        .lua
        .load(script)
        .eval()
}

fn lua_string(app: &App, script: &str) -> String {
    lua(app, script)
        .ok()
        .and_then(|value| value.as_string().map(|s| s.to_string_lossy()))
        .unwrap_or_default()
}

/// Pressing the buy skill opens the original task-12 dialog with the equipped fish's weight, length
/// and weighted refund; OK sends the unchanged `UseDoodadSkill`, cancel and `INTERACTION_END` send
/// nothing.
#[test]
fn the_buy_dialog_opens_and_ok_sends_the_skill() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(
        game,
        fish_inventory(FISH_TEMPLATE, WEIGHT, LENGTH),
        BUY_SKILL,
    );
    app.update();
    assert!(
        sent(&mut app).is_empty(),
        "a fish sale was sent without confirmation"
    );
    assert_eq!(lua_string(&app, "return HUD.fish_state()"), "open");
    assert!(
        lua(&app, "return HUD.fish_visible()")
            .unwrap()
            .as_boolean()
            .unwrap_or(false)
    );
    lua(
        &app,
        "assert(FISH_INFO.type==DLG_TASK_BUY_FISH and FISH_INFO.buttonType==0 and FISH_INFO.isModal); \
         assert(FISH_INFO.itemInfo.refund==133689 and FISH_INFO.itemInfo.weight==500 and FISH_INFO.itemInfo.length==280); \
         assert(FISH_INFO.itemType==nil and FISH_INFO.itemCount==nil); \
         assert(type(FISH_INFO.itemInfo.name)=='string' and #FISH_INFO.itemInfo.name > 0); return true",
    )
    .unwrap();

    lua(&app, "X2DialogManager:OnOK(HUD.fish_id())").unwrap();
    app.update();
    assert!(
        matches!(
            sent(&mut app).as_slice(),
            [Command::UseDoodadSkill {
                skill_id: BUY_SKILL,
                doodad_obj_id: 2075
            }]
        ),
        "OK did not send the sold skill"
    );
    assert_eq!(lua_string(&app, "return HUD.fish_state()"), "");
}

/// Cancel and the interaction end close the dialog without a packet.
#[test]
fn the_fish_dialog_cancel_and_interaction_end_send_nothing() {
    if !aa_cry::pak_available() {
        return;
    }
    if game().is_none() {
        return;
    }
    for cancel in [true, false] {
        let Some(game) = game() else { return };
        let mut app = app(
            game,
            fish_inventory(FISH_TEMPLATE, WEIGHT, LENGTH),
            BUY_SKILL,
        );
        app.update();
        assert!(sent(&mut app).is_empty());
        if cancel {
            lua(&app, "X2DialogManager:OnCancel(HUD.fish_id())").unwrap();
        } else {
            app.world()
                .get_non_send::<UiRuntime>()
                .unwrap()
                .fire_event("INTERACTION_END", ());
        }
        app.update();
        assert!(sent(&mut app).is_empty(), "cancel {cancel} sent a skill");
        assert_eq!(
            lua_string(&app, "return HUD.fish_state()"),
            "",
            "cancel {cancel} left the dialog"
        );
    }
}

/// `FUN_393b7e40`: walking out of the interaction distance, or the doodad going away, ends it.
#[test]
fn the_fish_dialog_ends_out_of_reach_or_with_the_doodad() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(
        game,
        fish_inventory(FISH_TEMPLATE, WEIGHT, LENGTH),
        BUY_SKILL,
    );
    app.update();
    assert_eq!(lua_string(&app, "return HUD.fish_state()"), "open");
    app.world()
        .get_non_send::<HudBridge>()
        .unwrap()
        .0
        .borrow_mut()
        .pos = Vec3::new(1000.0, 1000.0, 100.0);
    app.update();
    assert!(sent(&mut app).is_empty());
    assert_eq!(lua_string(&app, "return HUD.fish_state()"), "");

    // reopen, then despawn the doodad
    app.world()
        .get_non_send::<HudBridge>()
        .unwrap()
        .0
        .borrow_mut()
        .pos = Vec3::new(10.0, 10.0, 100.0);
    app.world()
        .get_non_send::<HudBridge>()
        .unwrap()
        .0
        .borrow_mut()
        .dynamic_actions
        .request(1);
    app.update();
    assert_eq!(lua_string(&app, "return HUD.fish_state()"), "open");
    app.world_mut()
        .resource_mut::<Doodads>()
        .by_id
        .remove(&2075);
    app.update();
    assert!(sent(&mut app).is_empty());
    assert_eq!(lua_string(&app, "return HUD.fish_state()"), "");
}

/// No item on the back -> the native 0x169 (361) through the existing error path, no dialog.
#[test]
fn no_back_item_shows_store_backpack_nogoods() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, inventory(None), BUY_SKILL);
    app.update();
    let errors: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<crate::hud::ShowError>>()
        .drain()
        .map(|error| error.0)
        .collect();
    assert_eq!(errors, [361]);
    assert!(sent(&mut app).is_empty());
    assert_eq!(lua_string(&app, "return HUD.fish_state()"), "");
}

/// The convert skill uses task 13 and the same item info (including the refund).
#[test]
fn the_convert_dialog_opens_task_13() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(
        game,
        fish_inventory(FISH_TEMPLATE, WEIGHT, LENGTH),
        CONVERT_SKILL,
    );
    app.update();
    assert!(sent(&mut app).is_empty());
    lua(
        &app,
        "assert(FISH_INFO.type==DLG_TASK_CONVERT_FISH); assert(FISH_INFO.itemInfo.refund==133689); \
         assert(FISH_INFO.itemInfo.weight==500 and FISH_INFO.itemInfo.length==280); \
         assert(FISH_INFO.itemCount==nil and FISH_INFO.itemType==nil)",
    )
    .unwrap();
    lua(&app, "X2DialogManager:OnOK(HUD.fish_id())").unwrap();
    app.update();
    assert!(matches!(
        sent(&mut app).as_slice(),
        [Command::UseDoodadSkill {
            skill_id: CONVERT_SKILL,
            ..
        }]
    ));
}
