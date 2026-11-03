use super::*;

use std::collections::HashSet;

use aa_net::Event;
use aa_net::model::Item;
use aa_net::state::{Doodads, Units};

use crate::hud::api::{HudBridge, Shared};
use crate::hud::doodad::{Focus, Views, execute, world_doodad};
use crate::hud::dynamic_actions::{Shown, Source};

const BUY_SKILL: u32 = 21_904;
const CONVERT_SKILL: u32 = 22_201;
const FISH_STAND_GROUP: u32 = 17_262;
const FISH_TEMPLATE: u32 = 27_457;

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

fn items(game: &GameData, kind: FishKind, func: u32) -> Option<&HashSet<u32>> {
    game.doodad.fish_items(kind, func)
}

/// `FUN_397cf5a0` / `FUN_397cf660`: the three 1.2 errors and the buy refund.
#[test]
fn the_fish_stand_checks_match_the_native_errors() {
    let Some(game) = game() else { return };
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
    assert_eq!(
        check(
            &game,
            items(&game, buy.kind, buy.func_id),
            &inventory(None),
            buy.kind
        ),
        Err(361)
    );
    assert_eq!(
        check(
            &game,
            items(&game, buy.kind, buy.func_id),
            &inventory(Some(23_605)),
            buy.kind
        ),
        Err(225),
        "the buy list is checked before the item kind (FUN_397cf5a0)"
    );
    assert_eq!(
        check(
            &game,
            items(&game, buy.kind, buy.func_id),
            &inventory(Some(28_768)),
            buy.kind
        ),
        Err(225),
        "a type-6 backpack the stand does not handle"
    );
    let sale = check(
        &game,
        items(&game, buy.kind, buy.func_id),
        &inventory(Some(FISH_TEMPLATE)),
        buy.kind,
    )
    .unwrap();
    assert_eq!((sale.template, sale.refund), (FISH_TEMPLATE, 150_000));
    // convert: a buy-only fish is not in the convert list
    assert_eq!(
        check(
            &game,
            items(&game, convert.kind, convert.func_id),
            &inventory(Some(27_602)),
            convert.kind
        ),
        Err(225)
    );
    assert_eq!(
        check(
            &game,
            items(&game, convert.kind, convert.func_id),
            &inventory(Some(23_605)),
            convert.kind
        ),
        Err(88),
        "convert checks the item kind before the list (FUN_397cf660)"
    );
    assert_eq!(
        check(
            &game,
            items(&game, convert.kind, convert.func_id),
            &inventory(Some(FISH_TEMPLATE)),
            convert.kind
        )
        .unwrap()
        .refund,
        0
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
                generation: 1,
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

/// Pressing the buy skill opens the original task-12 dialog with the equipped fish's info; OK
/// sends the unchanged `UseDoodadSkill`, cancel and `INTERACTION_END` send nothing.
#[test]
fn the_buy_dialog_opens_and_ok_sends_the_skill() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, inventory(Some(FISH_TEMPLATE)), BUY_SKILL);
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
        "assert(FISH_INFO.type==DLG_TASK_BUY_FISH and FISH_INFO.buttonType==DBT_OK_CANCEL and FISH_INFO.isModal); assert(FISH_INFO.itemInfo.refund==150000 and FISH_INFO.itemCount==1); assert(type(FISH_INFO.itemInfo.name)=='string' and #FISH_INFO.itemInfo.name > 0); return true",
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
    assert!(
        matches!(lua(&app, "return HUD.fish_state()").unwrap(), mlua::Value::String(s) if s.to_string_lossy().is_empty())
    );
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
        let mut app = app(game, inventory(Some(FISH_TEMPLATE)), BUY_SKILL);
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

/// The convert skill uses task 13 and the same item info without a refund.
#[test]
fn the_convert_dialog_opens_task_13() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, inventory(Some(FISH_TEMPLATE)), CONVERT_SKILL);
    app.update();
    assert!(sent(&mut app).is_empty());
    lua(
        &app,
        "assert(FISH_INFO.type==DLG_TASK_CONVERT_FISH); assert(FISH_INFO.itemInfo.refund==nil)",
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
