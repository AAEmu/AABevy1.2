use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use aa_net::interaction::{Interact, InteractEvent};
use aa_net::model::{Attachment, Movement, Unit, UnitKind};
use aa_net::slave::SlaveEvent;
use aa_net::slave_control::SlaveCommand;
use aa_net::state::Units;
use aa_net::wire::Reader;
use aa_net::{Command, Event, NetCommand, NetEvent, Pos};
use web_time::Instant;

use super::*;
use crate::hud::api::{HudBridge, HudData, Request, Shared};

fn slave(obj_id: u32, tl_id: u16, at: Vec3) -> Unit {
    let mut unit = crate::hud::fixture_spawn().unit;
    unit.obj_id = obj_id;
    unit.name = format!("Cart {obj_id}");
    unit.kind = UnitKind::Slave {
        id: 700,
        tl_id,
        template_id: 60,
        summoner_id: 100,
    };
    // The Farm Wagon of the 2026-10-28 capture (slave template 60, model 1008).
    unit.model_id = 1008;
    unit.pos = Pos::new(at.x, at.y, at.z);
    unit
}

/// The HUD's game data with actor-model radii only (no database).
fn game_with_radii(radii: &[(u32, f32)]) -> crate::hud::data::GameData {
    let mut game = crate::hud::data::GameData::default();
    game.model_radii.extend(radii.iter().copied());
    game
}

fn setup(slaves: &[Unit]) -> (Shared, Units) {
    let state: Shared = Rc::new(RefCell::new(HudData::default()));
    let own = crate::hud::fixture_spawn();
    crate::hud::mounted::receive(
        &state,
        &Event::CharacterSelected(Box::new(own.state.clone())),
    );
    crate::hud::mounted::receive(&state, &Event::PlayerSpawn(Box::new(own)));
    state.borrow_mut().pos = Vec3::ZERO;
    let mut units = Units::default();
    for slave in slaves {
        units.by_id.insert(slave.obj_id, slave.clone());
        crate::hud::mounted::receive(&state, &Event::UnitSpawned(Box::new(slave.clone())));
    }
    (state, units)
}

fn send(state: &Shared, units: &Units) -> Vec<Command> {
    let mut app = App::new();
    app.insert_non_send(HudBridge(state.clone()))
        .init_resource::<NextState<crate::Screen>>()
        .insert_resource(units.clone())
        .init_resource::<aa_net::state::ActionBar>()
        .init_resource::<aa_net::state::Inventory>()
        .init_resource::<aa_net::state::Target>()
        .init_resource::<aa_net::state::Skills>()
        .init_resource::<aa_net::state::Buffs>()
        .init_resource::<crate::hud::InFlight>()
        .add_message::<NetCommand>()
        .add_message::<crate::hud::alerts::UseFailed>()
        .add_message::<crate::ingame::ground_target::SkillPressed>()
        .add_systems(Update, crate::hud::api::handle_requests);
    app.update();
    app.world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|message| message.0)
        .collect()
}

fn bind(state: &Shared, units: &Units, target: u32) -> Outcome {
    let mut data = state.borrow_mut();
    super::bind(&mut data, units, target)
}

fn runtime() -> UiRuntime {
    let rt = UiRuntime::new("en_us", 800., 600.).expect("UI runtime");
    rt.lua
        .load(
            "X2Locale={LocalizeUiText=function(_,_,key) return 'text:'..key end};ERROR_MSG=1;\
             sys={};local w=UIParent:CreateWidget('emptywidget','slaveProbe','UIParent');\
             w:RegisterEvent('SYSMSG');w:SetHandler('OnEvent',function(_,_,text) sys[#sys+1]=text end)",
        )
        .exec()
        .expect("SYSMSG probe");
    rt
}

fn install_discard(rt: &UiRuntime, state: &Shared) {
    rt.lua
        .globals()
        .set("X2SiegeWeapon", rt.lua.create_table().expect("table"))
        .expect("namespace");
    install(&rt.lua, state).expect("DiscardSiegeWeapon");
}

#[test]
fn the_interact_key_targets_only_slave_units() {
    let mut npc = crate::hud::fixture_spawn().unit;
    npc.obj_id = 30;
    npc.kind = UnitKind::Npc {
        template_id: 1,
        owner_id: 0,
    };
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0)), npc]);
    assert_eq!(action_target(&units, Some(20)), Some(20));
    assert_eq!(action_target(&units, Some(30)), None);
    assert_eq!(action_target(&units, None), None);
    assert_eq!(state.borrow().requests.len(), 0);
}

fn npc(obj_id: u32) -> Unit {
    let mut npc = crate::hud::fixture_spawn().unit;
    npc.obj_id = obj_id;
    npc.kind = UnitKind::Npc {
        template_id: 1,
        owner_id: 0,
    };
    npc
}

#[test]
fn the_interact_key_never_acts_on_a_slave() {
    let (_, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0)), npc(30)]);
    assert_eq!(action_target(&units, None), None);
    assert_eq!(action_target(&units, Some(30)), None);
    assert_eq!(action_target(&units, Some(20)), Some(20));
}

fn skill_list(obj_id: u32, skills: &[u32]) -> Event {
    Event::Interaction(InteractEvent::SkillList {
        npc_obj_id: obj_id,
        obj_id: 0,
        extra_info: 1,
        pick_id: 0,
        mouse_button: 3,
        skills: skills.to_vec(),
        interactable: true,
        modifier_keys: 0,
    })
}

fn receive(state: &Shared, units: &Units, event: &Event) -> Outcome {
    super::receive(&mut state.borrow_mut(), units, event)
}

#[test]
fn a_right_click_on_a_cart_sends_change_target_then_start_interaction() {
    let (state, units) = setup(&[slave(0x5f3c, 8, Vec3::new(2.0, 0.0, 0.0))]);
    let obj_id = action_target(&units, Some(0x5f3c)).unwrap();
    assert_eq!(
        super::interact(&mut state.borrow_mut(), &units, None, obj_id),
        Outcome::Sent
    );
    let commands = send(&state, &units);
    assert_eq!(commands.len(), 2, "{commands:?}");
    assert!(matches!(&commands[0], Command::Target { obj_id: 0x5f3c }));
    match &commands[1] {
        Command::Interact(Interact::Start {
            npc_obj_id: 0x5f3c,
            obj_id: 0,
            extra_info: 1,
            pick_id: 0,
            mouse_button: 3,
            modifier_keys: 0,
        }) => {}
        other => panic!("{other:?}"),
    }
    assert!(
        commands
            .iter()
            .all(|command| !matches!(command, Command::Slave(_))),
        "a right click must not send CSBindSlave: {commands:?}"
    );
}

#[test]
fn a_cart_already_targeted_is_not_retargeted() {
    let (state, units) = setup(&[slave(0x5f3c, 8, Vec3::new(2.0, 0.0, 0.0))]);
    assert_eq!(
        super::interact(&mut state.borrow_mut(), &units, Some(0x5f3c), 0x5f3c),
        Outcome::Sent
    );
    assert!(matches!(
        state.borrow().requests.as_slice(),
        [Request::Interact(Interact::Start { .. })]
    ));
}

#[test]
fn the_occupy_skill_in_reach_sends_the_bind() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(2.99, 0.0, 0.0))]);
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Sent
    );
    let commands = send(&state, &units);
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert!(matches!(
        &commands[0],
        Command::Slave(SlaveCommand::Bind { tl_id: 8 })
    ));
}

#[test]
fn the_occupy_skill_reach_is_strictly_below_three_metres() {
    // FUN_39093860: `gap < limit`, so exactly 3.0 m is already out of reach.
    let (state, units) = setup(&[slave(20, 8, Vec3::new(3.0, 0.0, 0.0))]);
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Refused(0x9a)
    );
    assert!(send(&state, &units).is_empty());
    let (state, units) = setup(&[slave(20, 8, Vec3::new(2.999, 0.0, 0.0))]);
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Sent
    );
}

#[test]
fn the_reach_is_the_3d_surface_gap_of_the_two_spheres() {
    let origin = Vec3::ZERO;
    // |(4, 0, 3)| = 5 m between the centres: minus radii 0.5 + 0.5 = 4 m.
    assert!((surface_gap(origin, 0.5, Vec3::new(4.0, 0.0, 3.0), 0.5) - 4.0).abs() < 1e-5);
    // The col shape gap is not clamped at 0: overlapping spheres give a negative gap.
    assert!(surface_gap(origin, 1.0, Vec3::new(0.5, 0.0, 0.0), 1.0) < 0.0);
    // Strictness: exactly 3.0 is out of the 3.0 m interaction reach, 3.0 - epsilon is in.
    assert!(surface_gap(origin, 0.5, Vec3::new(4.0, 0.0, 0.0), 0.5) >= INTERACT_REACH);
    assert!(surface_gap(origin, 0.5, Vec3::new(3.999, 0.0, 0.0), 0.5) < INTERACT_REACH);
    // The mounted 3.5 m variant takes the same gap.
    assert!(surface_gap(origin, 0.5, Vec3::new(4.4, 0.0, 0.0), 0.5) < MOUNTED_INTERACT_REACH);
    assert!(surface_gap(origin, 0.5, Vec3::new(4.5, 0.0, 0.0), 0.5) >= MOUNTED_INTERACT_REACH);
}

#[test]
fn the_reach_is_measured_in_three_dimensions() {
    // 2.4 m along the ground and 1.8 m up: 3.0 m apart, out of reach; 1.7 m up is inside.
    let (state, units) = setup(&[slave(20, 8, Vec3::new(2.4, 0.0, 1.8))]);
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Refused(0x9a)
    );
    let (state, units) = setup(&[slave(20, 8, Vec3::new(2.4, 0.0, 1.7))]);
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Sent
    );
}

#[test]
fn a_skill_list_event_for_a_slave_reaches_the_bind_path() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    let mut app = App::new();
    app.insert_non_send(HudBridge(state.clone()))
        .insert_resource(units)
        .add_message::<NetEvent>()
        .add_systems(Update, super::track);
    app.world_mut()
        .write_message(NetEvent(skill_list(20, &[12076])));
    app.update();
    assert_eq!(
        state.borrow().requests,
        vec![Request::Slave(SlaveCommand::Bind { tl_id: 8 })]
    );
}

#[test]
fn the_occupy_skill_out_of_three_metres_sends_nothing() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(3.1, 0.0, 0.0))]);
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Refused(0x9a)
    );
    assert!(send(&state, &units).is_empty());
}

#[test]
fn the_occupy_skill_is_only_the_first_skill_and_other_skills_do_nothing() {
    for skills in [&[0][..], &[13863], &[19165], &[0, 12076], &[]] {
        let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
        assert_eq!(
            receive(&state, &units, &skill_list(20, skills)),
            Outcome::Silent,
            "{skills:?}"
        );
        assert!(send(&state, &units).is_empty(), "{skills:?}");
    }
}

#[test]
fn a_skill_list_for_an_npc_is_unchanged() {
    let (state, units) = setup(&[npc(30)]);
    assert_eq!(
        receive(&state, &units, &skill_list(30, &[12076])),
        Outcome::Silent
    );
    assert!(send(&state, &units).is_empty());
}

#[test]
fn a_skill_list_for_an_unknown_obj_id_is_silent() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    assert_eq!(
        receive(&state, &units, &skill_list(99, &[12076])),
        Outcome::Silent
    );
    assert!(send(&state, &units).is_empty());
}

#[test]
fn a_skill_list_for_a_dead_slave_is_refused_from_receive() {
    let mut cart = slave(20, 8, Vec3::new(1.0, 0.0, 0.0));
    cart.dead = true;
    let (state, units) = setup(&[cart]);
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Refused(0x144)
    );
    assert!(send(&state, &units).is_empty());
}

#[test]
fn dead_slave_is_refused_with_the_native_message() {
    let mut cart = slave(20, 8, Vec3::new(1.0, 0.0, 0.0));
    cart.dead = true;
    let (state, units) = setup(&[cart]);
    assert_eq!(bind(&state, &units, 20), Outcome::Refused(0x144));
    assert!(state.borrow().requests.is_empty());
    assert!(
        send(&state, &units).is_empty(),
        "a refused bind sends nothing"
    );
}

#[test]
fn another_bound_slave_is_refused_with_unbind_first() {
    let (state, units) = setup(&[
        slave(20, 8, Vec3::new(1.0, 0.0, 0.0)),
        slave(21, 9, Vec3::new(2.0, 0.0, 0.0)),
    ]);
    crate::hud::mounted::receive(
        &state,
        &Event::Slave(SlaveEvent::Bound {
            master_id: 47,
            obj_id: 20,
        }),
    );
    assert_eq!(bind(&state, &units, 21), Outcome::Refused(0x66));
    assert!(state.borrow().requests.is_empty());
}

#[test]
fn the_bound_slave_itself_is_silent() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    crate::hud::mounted::receive(
        &state,
        &Event::Slave(SlaveEvent::Bound {
            master_id: 47,
            obj_id: 20,
        }),
    );
    assert_eq!(bind(&state, &units, 20), Outcome::Silent);
    assert!(state.borrow().requests.is_empty());
}

#[test]
fn the_seat_attachment_also_refuses_another_slave() {
    let (state, units) = setup(&[
        slave(20, 8, Vec3::new(1.0, 0.0, 0.0)),
        slave(21, 9, Vec3::new(2.0, 0.0, 0.0)),
    ]);
    crate::hud::mounted::receive(
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 0,
        },
    );
    assert_eq!(bind(&state, &units, 21), Outcome::Refused(0x66));
    assert_eq!(bind(&state, &units, 20), Outcome::Silent);
}

#[test]
fn resurrection_ends_the_seat_and_lets_the_player_board_again() {
    let (state, units) = setup(&[
        slave(20, 8, Vec3::new(1.0, 0.0, 0.0)),
        slave(21, 9, Vec3::new(2.0, 0.0, 0.0)),
    ]);
    crate::hud::mounted::receive(
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 0,
        },
    );
    bound(&state, 20);
    assert_eq!(bind(&state, &units, 21), Outcome::Refused(0x66));
    // Player::apply ends the seat on resurrection; no separate UnitDetached arrives.
    crate::hud::mounted::receive(
        &state,
        &Event::Resurrected {
            obj_id: 100,
            pos: Pos::default(),
            yaw: 0.0,
        },
    );
    assert!(super::current_slave(&state.borrow(), &units).is_none());
    assert!(state.borrow().mounted.player_attachment().is_none());
    assert!(state.borrow().mounted.bound_slave().is_none());
    // The old cart can be right-clicked through the ordinary interaction path again.
    assert_eq!(
        super::interact(&mut state.borrow_mut(), &units, None, 20),
        Outcome::Sent
    );
    let commands = send(&state, &units);
    assert_eq!(commands.len(), 2, "{commands:?}");
    assert!(matches!(&commands[0], Command::Target { obj_id: 20 }));
    assert!(matches!(
        &commands[1],
        Command::Interact(Interact::Start { npc_obj_id: 20, .. })
    ));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::Slave(_))),
        "the stale seat must not send a slave command: {commands:?}"
    );
    // Another cart can be boarded: the old bound fallback no longer answers 0x66.
    assert_eq!(bind(&state, &units, 21), Outcome::Sent);
}

#[test]
fn a_pending_bind_is_refused_with_already_requested() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
    assert_eq!(bind(&state, &units, 20), Outcome::Refused(0x13));
    assert_eq!(state.borrow().requests.len(), 1);
}

#[test]
fn a_right_click_while_a_bind_is_pending_is_refused_with_already_requested() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(2.0, 0.0, 0.0))]);
    let obj_id = action_target(&units, Some(20)).unwrap();
    assert_eq!(
        super::interact(&mut state.borrow_mut(), &units, Some(20), obj_id),
        Outcome::Sent
    );
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Sent
    );
    // The second click sends 0x068 again; its 0x07b answer meets the pending first bind.
    assert_eq!(
        super::interact(&mut state.borrow_mut(), &units, Some(20), obj_id),
        Outcome::Sent
    );
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Refused(0x13)
    );
    assert_eq!(
        state
            .borrow()
            .requests
            .iter()
            .filter(|request| matches!(request, Request::Slave(SlaveCommand::Bind { .. })))
            .count(),
        1
    );
}

#[test]
fn the_bind_reach_is_strictly_below_four_metres() {
    // FUN_39346290 reuses the reach predicate: `gap < 4.0`, so exactly 4.0 m is refused.
    let (state, units) = setup(&[slave(20, 8, Vec3::new(4.0, 0.0, 0.0))]);
    assert_eq!(bind(&state, &units, 20), Outcome::Refused(0x9a));
    assert!(state.borrow_mut().requests.is_empty());
    let (state, units) = setup(&[slave(20, 8, Vec3::new(3.999, 0.0, 0.0))]);
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
}

#[test]
fn the_bind_reach_subtracts_the_scaled_model_radii() {
    // Player 0.5 m and wagon 0.5 m: centres 5.0 m apart leave exactly the 4.0 m limit.
    let (state, units) = setup(&[slave(20, 8, Vec3::new(5.0, 0.0, 0.0))]);
    state.borrow_mut().player_radius = 0.5;
    state
        .borrow()
        .game
        .0
        .set(game_with_radii(&[(1008, 0.5)]))
        .ok()
        .expect("game data");
    assert_eq!(bind(&state, &units, 20), Outcome::Refused(0x9a));

    let (state, units) = setup(&[slave(20, 8, Vec3::new(4.999, 0.0, 0.0))]);
    state.borrow_mut().player_radius = 0.5;
    state
        .borrow()
        .game
        .0
        .set(game_with_radii(&[(1008, 0.5)]))
        .ok()
        .expect("game data");
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
}

/// The 2026-10-28 capture (Farm Wagon, slave template 60, model 1008): the 3.1022 m placement
/// bound in 1.2 with no walking. The wagon is a `VehicleModel`, so its actor-model radius is 0;
/// every 1.2 player model still leaves the surface gap under 3.0 m.
#[test]
#[ignore = "requires the game pak and the game database (AA_PAK, AA_DB)"]
fn the_captured_farm_wagon_reaches_pass_with_the_real_model_radii()
-> Result<(), Box<dyn std::error::Error>> {
    let pak = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.into());
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    if !std::path::Path::new(&pak).exists() || !std::path::Path::new(&path).exists() {
        eprintln!("skipping: the game pak {pak} or database {path} is not there");
        return Ok(());
    }
    let game = crate::hud::data::GameData::load(&path)?;
    // The capture's positions are f64; cast at the same place the game does.
    let captured = |at: [f64; 3]| Vec3::new(at[0] as f32, at[1] as f32, at[2] as f32);
    let wagon = captured([4318.0, 4123.0, 97.8199615]);
    let wagon_radius = game.model_size(1008, 1.0);
    for (player, measured) in [
        (captured([4318.0, 4119.8984375, 97.75827026367188]), 3.1022),
        (
            captured([4318.0, 4120.099609375, 97.77352142333984]),
            2.9008,
        ),
        (captured([4318.0, 4121.0, 97.48428344726562]), 2.0280),
    ] {
        for model in 10..=17 {
            let gap = surface_gap(player, game.model_size(model, 1.0), wagon, wagon_radius);
            assert!(
                gap < INTERACT_REACH,
                "player model {model} at {measured} m: gap {gap} m"
            );
        }
    }
    // The same through the shipped check: the 3.1022 m placement binds.
    let (state, units) = setup(&[slave(20, 8, wagon)]);
    state.borrow_mut().pos = captured([4318.0, 4119.8984375, 97.75827026367188]);
    state.borrow_mut().player_radius = game.model_size(10, 1.0);
    state.borrow().game.0.set(game).ok().expect("game data");
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Sent
    );
    Ok(())
}

#[test]
fn only_a_mate_seat_switches_the_interaction_reach_to_three_point_five_metres() {
    let mut mate = crate::hud::expansion::fixture::spawn().unit;
    mate.obj_id = 31;
    let (state, units) = setup(&[slave(30, 8, Vec3::ZERO), mate]);
    assert_eq!(interact_reach(&state.borrow()), INTERACT_REACH);
    let attach = |mount| {
        crate::hud::mounted::receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount,
                point: 1,
                reason: 0,
            },
        )
    };
    // A slave seat is not the +0x3c26 mounted state (unverified), so the reach stays 3 m.
    attach(30);
    assert_eq!(interact_reach(&state.borrow()), INTERACT_REACH);
    // The reach is measured from the seat, so the checks do not use the stale HUD point.
    assert_eq!(
        player_centre(&state.borrow(), &units),
        Vec3::new(0.0, 0.0, 0.0)
    );
    // Riding a mate (any unit that is not a slave) is mounted.
    attach(31);
    assert_eq!(interact_reach(&state.borrow()), MOUNTED_INTERACT_REACH);
}

#[test]
fn the_reach_while_attached_uses_the_slaves_position_not_the_stale_hud_one() {
    // The 2026-10-28 replay answered a while-bound right click from a stale on-foot position
    // ~7 m away; the server had moved the cart, not the rider.
    let (state, units) = setup(&[slave(20, 8, Vec3::new(4318.0, 4123.0, 97.82))]);
    crate::hud::mounted::receive(
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 0,
        },
    );
    state.borrow_mut().pos = Vec3::new(4304.0, 4116.0, 97.0);
    assert_eq!(
        receive(&state, &units, &skill_list(20, &[12076])),
        Outcome::Silent,
        "already bound to this slave, but the stale point must not refuse it"
    );
    assert!(state.borrow_mut().requests.is_empty());
}

#[test]
fn a_right_click_while_bound_sends_only_the_target_change() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(4318.0, 4123.0, 97.82))]);
    bound(&state, 20);
    assert_eq!(
        super::interact(&mut state.borrow_mut(), &units, None, 20),
        Outcome::Sent
    );
    let commands = send(&state, &units);
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert!(matches!(&commands[0], Command::Target { obj_id: 20 }));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::Interact(_) | Command::Slave(_))),
        "a right click on the bound slave must not send 0x068 or 0x031: {commands:?}"
    );
    // Already targeted: the native client has nothing left to send.
    assert_eq!(
        super::interact(&mut state.borrow_mut(), &units, Some(20), 20),
        Outcome::Silent
    );
    assert!(state.borrow().requests.is_empty());
}

#[test]
fn a_right_click_on_a_slave_we_are_seated_on_sends_no_interaction() {
    let (state, units) = setup(&[slave(20, 8, Vec3::ZERO)]);
    crate::hud::mounted::receive(
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 2,
            reason: 0,
        },
    );
    assert_eq!(
        super::interact(&mut state.borrow_mut(), &units, None, 20),
        Outcome::Sent
    );
    assert!(matches!(
        state.borrow().requests.as_slice(),
        [Request::Target { obj_id: 20 }]
    ));
}

#[test]
fn refusal_uses_the_original_system_message_text() -> mlua::Result<()> {
    let lua = Lua::new();
    lua.load("X2Locale={LocalizeUiText=function(_,_,key) return 'text:'..key end};ERROR_MSG=1")
        .exec()?;
    assert_eq!(text(&lua, 0x9a), "text:TOO_FAR_AWAY");
    assert_eq!(text(&lua, 0x144), "text:SLAVE_CANNOT_BIND_WHILE_IS_DEAD");
    assert_eq!(text(&lua, 0x67), "text:SLAVE_SHUT_OFF_TO_UNBIND");
    Ok(())
}

fn bound(state: &Shared, obj_id: u32) {
    crate::hud::mounted::receive(
        state,
        &Event::Slave(SlaveEvent::Bound {
            master_id: 47,
            obj_id,
        }),
    );
}

fn discard(state: &Shared) -> Outcome {
    super::discard(&mut state.borrow_mut())
}

#[test]
fn discard_siege_weapon_sends_unbind_for_the_bound_slave() {
    let (state, _units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    bound(&state, 20);
    assert_eq!(discard(&state), Outcome::Sent);
    assert_eq!(
        state.borrow().requests,
        vec![Request::Slave(SlaveCommand::Unbind { tl_id: 8 })]
    );
}

#[test]
fn discard_uses_the_seated_slave_without_a_bound_reply() {
    let (state, _units) = setup(&[
        slave(20, 8, Vec3::new(1.0, 0.0, 0.0)),
        slave(21, 9, Vec3::new(2.0, 0.0, 0.0)),
    ]);
    assert_eq!(discard(&state), Outcome::Silent, "neither seated nor bound");
    crate::hud::mounted::receive(
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 21,
            point: 1,
            reason: 0,
        },
    );
    assert_eq!(discard(&state), Outcome::Sent);
    // the seat wins over a SCSlaveBound naming another slave
    bound(&state, 20);
    assert_eq!(discard(&state), Outcome::Sent);
    assert_eq!(
        state.borrow().requests,
        vec![
            Request::Slave(SlaveCommand::Unbind { tl_id: 9 }),
            Request::Slave(SlaveCommand::Unbind { tl_id: 9 }),
        ]
    );
}

#[test]
fn discard_is_refused_while_the_slave_moves_faster_than_five() {
    let (state, _units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    bound(&state, 20);
    let mut movement = Movement::unit(Pos::default(), 0.0, 1);
    movement.velocity = [6 * 1024, 0, 0];
    crate::hud::mounted::receive(
        &state,
        &Event::UnitMoved {
            obj_id: 20,
            movement,
        },
    );
    assert_eq!(discard(&state), Outcome::Refused(0x67));
    assert!(state.borrow().requests.is_empty());

    let mut movement = Movement::unit(Pos::default(), 0.0, 2);
    movement.velocity = [5 * 1024, 0, 0];
    crate::hud::mounted::receive(
        &state,
        &Event::UnitMoved {
            obj_id: 20,
            movement,
        },
    );
    assert_eq!(discard(&state), Outcome::Sent);
    assert_eq!(
        state.borrow().requests,
        vec![Request::Slave(SlaveCommand::Unbind { tl_id: 8 })]
    );
}

fn hex(raw: &str) -> Vec<u8> {
    raw.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

#[test]
fn a_real_type_2_body_uses_the_vehicle_velocity_scale() {
    // 2026-10-29 capture, case d_reverse, a type-2 body at 4.00 m/s: velocity
    // [-869, 4240, 593] at 30/32767 m/s per unit.
    let body = hex("c9 84 00 02 1f 43 11 00 00 4d d7 51 69 8f 79 68
         04 05 9b fc 90 10 51 02 8c 09 b4 09 4f 81 00 00
         00 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
    let mut r = Reader::new(&body);
    assert_eq!(r.bc().unwrap(), 0x84C9);
    let movement = Movement::read(&mut r).unwrap();
    assert_eq!(movement.move_type, 2);
    assert_eq!(movement.velocity, [-869, 4240, 593]);

    let scale = aa_net::vehicle::VEL_VEHICLE / 32767.0;
    let expected = ((869f32).powi(2) + (4240f32).powi(2) + (593f32).powi(2)) * scale * scale;
    let squared = crate::hud::mounted::movement_speed_squared(Some(&movement));
    assert!((squared - expected).abs() < 1e-3, "{squared} vs {expected}");
    // the old 1/1024 scaling would read 4.27 m/s instead of 4.00
    let wrong = ((869f32).powi(2) + (4240f32).powi(2) + (593f32).powi(2)) / (1024.0 * 1024.0);
    assert!(squared < wrong - 0.5, "{squared} vs {wrong}");

    let (state, _units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    bound(&state, 20);
    crate::hud::mounted::receive(
        &state,
        &Event::UnitMoved {
            obj_id: 20,
            movement: movement.clone(),
        },
    );
    assert_eq!(discard(&state), Outcome::Sent);

    // 4.8 m/s on the type-2 scale is below the bound; 1/1024 would read 5.12 and refuse
    let mut fast = movement;
    fast.velocity = [5243, 0, 0];
    crate::hud::mounted::receive(
        &state,
        &Event::UnitMoved {
            obj_id: 20,
            movement: fast,
        },
    );
    assert_eq!(discard(&state), Outcome::Sent);
}

#[test]
fn a_ship_driver_leave_sends_the_final_helm_zero_before_the_unbind() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    bound(&state, 20);
    assert_eq!(discard(&state), Outcome::Sent);
    let mut app = App::new();
    app.insert_non_send(HudBridge(state.clone()))
        .init_resource::<NextState<crate::Screen>>()
        .insert_resource(units.clone())
        .init_resource::<aa_net::state::ActionBar>()
        .init_resource::<aa_net::state::Inventory>()
        .init_resource::<aa_net::state::Target>()
        .init_resource::<aa_net::state::Skills>()
        .init_resource::<aa_net::state::Buffs>()
        .init_resource::<crate::hud::InFlight>()
        .insert_resource(crate::ingame::LocalMount(Some(crate::ingame::LocalSeat {
            mount: 20,
            point: 1,
            drives: true,
            cart: false,
            ship: true,
            speed: Default::default(),
            lift: 0.0,
        })))
        .init_resource::<crate::ingame::ShipDrive>()
        .add_message::<NetCommand>()
        .add_message::<crate::hud::alerts::UseFailed>()
        .add_message::<crate::ingame::ground_target::SkillPressed>()
        .add_systems(Update, crate::hud::api::handle_requests);
    app.update();
    let commands: Vec<Command> = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|message| message.0)
        .collect();
    assert_eq!(commands.len(), 2, "the final 0/0 then CSDiscardSlave, nothing after");
    let Command::ShipControl(request) = &commands[0] else {
        panic!("the final helm request is not first");
    };
    assert_eq!(
        (request.slave_obj_id(), request.throttle(), request.steering()),
        (20, 0, 0)
    );
    let Command::Slave(SlaveCommand::Unbind { tl_id }) = &commands[1] else {
        panic!("the discard request is not second");
    };
    assert_eq!(*tl_id, 8);
}

#[test]
fn the_captured_leave_while_moving_at_3_54_mps_sends_the_unbind() {
    // 2026-10-29 capture, case f_leave: the last cart velocity before
    // CSDiscardSlave 0x032 was 3.54 m/s (measured 3.63), below the 5 m/s bound.
    let (state, _units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    bound(&state, 20);
    let mut movement = Movement::unit(Pos::default(), 0.0, 1);
    movement.velocity = [(3.54 * 1024.0) as i16, 0, 0];
    crate::hud::mounted::receive(
        &state,
        &Event::UnitMoved {
            obj_id: 20,
            movement,
        },
    );
    assert_eq!(discard(&state), Outcome::Sent);
    assert_eq!(
        state.borrow().requests,
        vec![Request::Slave(SlaveCommand::Unbind { tl_id: 8 })]
    );
}

#[test]
fn a_stale_replicated_speed_no_longer_refuses_the_unbind() {
    let (state, _units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    bound(&state, 20);
    let mut movement = Movement::unit(Pos::default(), 0.0, 1);
    movement.velocity = [6 * 1024, 0, 0];
    crate::hud::mounted::receive(
        &state,
        &Event::UnitMoved {
            obj_id: 20,
            movement,
        },
    );
    assert_eq!(
        discard_at(&mut state.borrow_mut(), Instant::now()),
        Outcome::Refused(0x67)
    );
    let now = Instant::now() + crate::hud::mounted::SPEED_STALE_AFTER + Duration::from_millis(1);
    assert_eq!(discard_at(&mut state.borrow_mut(), now), Outcome::Sent);
    assert_eq!(
        state.borrow().requests,
        vec![Request::Slave(SlaveCommand::Unbind { tl_id: 8 })]
    );
}

#[test]
#[ignore = "requires the original UI (game pak)"]
fn discard_siege_weapon_lua_wrapper_queues_the_same_unbind()
-> Result<(), Box<dyn std::error::Error>> {
    let (state, _units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    bound(&state, 20);
    let rt = runtime();
    install_discard(&rt, &state);
    rt.lua.load("X2SiegeWeapon:DiscardSiegeWeapon()").exec()?;
    assert_eq!(
        state.borrow().requests,
        vec![Request::Slave(SlaveCommand::Unbind { tl_id: 8 })]
    );
    Ok(())
}

#[test]
#[ignore = "requires the original UI (game pak)"]
fn fast_slave_unbind_refusal_shows_sysmsg_through_the_lua_wrapper()
-> Result<(), Box<dyn std::error::Error>> {
    let (state, _units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    bound(&state, 20);
    let mut movement = Movement::unit(Pos::default(), 0.0, 1);
    movement.velocity = [6 * 1024, 0, 0];
    crate::hud::mounted::receive(
        &state,
        &Event::UnitMoved {
            obj_id: 20,
            movement,
        },
    );
    let rt = runtime();
    install_discard(&rt, &state);
    rt.lua.load("X2SiegeWeapon:DiscardSiegeWeapon()").exec()?;
    assert!(state.borrow().requests.is_empty());
    assert_eq!(
        rt.lua.load("return sys[1]").eval::<String>()?,
        "text:SLAVE_SHUT_OFF_TO_UNBIND"
    );
    Ok(())
}

#[test]
fn leaving_the_seat_retires_the_bound_slave() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    crate::hud::mounted::receive(
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 2,
            reason: 0,
        },
    );
    bound(&state, 20);
    crate::hud::mounted::receive(
        &state,
        &Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
    );
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
}

#[test]
fn passenger_walk_off_is_throttled_to_1500_ms() {
    let mut view = crate::hud::mounted::View::default();
    let now = web_time::Instant::now();
    assert!(view.walk_off_throttle(now));
    assert!(!view.walk_off_throttle(now + Duration::from_millis(1499)));
    assert!(view.walk_off_throttle(now + Duration::from_millis(1500)));
}

fn walk_off_app(state: &Shared, units: &Units, point: u8, focus: bool, suspended: bool) -> App {
    let mut spawn = crate::hud::fixture_spawn();
    spawn.unit.attached_to = Some(Attachment { obj_id: 20, point, reason: 0 });
    let mut app = App::new();
    app.insert_non_send(HudBridge(state.clone()))
        .insert_resource(units.clone())
        .insert_resource(aa_net::state::Player { spawn: Some(spawn) })
        .insert_resource(crate::settings::Actions::default())
        .insert_resource(ButtonInput::<MouseButton>::default())
        .insert_resource(super::super::HudPointerOverUi(false))
        .insert_resource(super::super::HudKeyboardFocus(focus))
        .insert_resource(super::super::HudSuspended(suspended))
        .insert_resource(crate::ingame::teleport::Teleport::default())
        .add_systems(Update, walk_off);
    app
}

fn tap_walk(app: &mut App) {
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .tap_for_test("moveforward");
}

fn hold_mouse(app: &mut App, left: bool, right: bool) {
    let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
    if left {
        mouse.press(MouseButton::Left);
    }
    if right {
        mouse.press(MouseButton::Right);
    }
}

#[test]
fn both_mouse_buttons_walk_off_the_passenger_seat() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    let mut app = walk_off_app(&state, &units, 2, false, false);
    hold_mouse(&mut app, true, true);
    app.update();
    assert_eq!(
        state.borrow().requests,
        vec![Request::Slave(SlaveCommand::Unbind { tl_id: 8 })],
        "holding both world mouse buttons runs forward in the player controller"
    );
}

#[test]
fn one_mouse_button_alone_is_not_movement() {
    for (left, right) in [(true, false), (false, true)] {
        let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
        let mut app = walk_off_app(&state, &units, 2, false, false);
        hold_mouse(&mut app, left, right);
        app.update();
        assert!(
            state.borrow().requests.is_empty(),
            "left {left}, right {right}: one button only turns the camera"
        );
    }
}

#[test]
fn both_mouse_buttons_over_the_hud_do_not_walk_off() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    let mut app = walk_off_app(&state, &units, 2, false, false);
    app.world_mut()
        .resource_mut::<super::super::HudPointerOverUi>()
        .0 = true;
    hold_mouse(&mut app, true, true);
    app.update();
    assert!(
        state.borrow().requests.is_empty(),
        "the player controller ignores world buttons held over a HUD widget"
    );
}

#[test]
fn walk_off_needs_a_passenger_seat_and_free_keyboard_focus() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    let mut app = walk_off_app(&state, &units, 1, false, false);
    tap_walk(&mut app);
    app.update();
    assert!(
        state.borrow().requests.is_empty(),
        "a driver seat does not walk off"
    );

    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    let mut app = walk_off_app(&state, &units, 2, true, false);
    tap_walk(&mut app);
    app.update();
    assert!(
        state.borrow().requests.is_empty(),
        "HUD keyboard focus is not movement input"
    );

    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    let mut app = walk_off_app(&state, &units, 2, false, true);
    tap_walk(&mut app);
    app.update();
    assert!(
        state.borrow().requests.is_empty(),
        "a suspended HUD cannot unbind"
    );

    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    let mut app = walk_off_app(&state, &units, 2, false, false);
    tap_walk(&mut app);
    app.update();
    assert_eq!(
        state.borrow().requests,
        vec![Request::Slave(SlaveCommand::Unbind { tl_id: 8 })]
    );
}

#[test]
fn every_walk_off_input_guard_blocks_the_unbind() {
    assert!(!input_blocked(false, false, false, false));
    assert!(input_blocked(true, false, false, false), "suspended HUD");
    assert!(input_blocked(false, true, false, false), "teleport loading");
    assert!(input_blocked(false, false, true, false), "HUD focus");
    assert!(
        input_blocked(false, false, false, true),
        "typing in a field"
    );
}

#[test]
fn a_locally_rejected_bind_clears_the_pending_request() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
    // RequestRejected comes from SlaveCommand::validate before the write, not from the server.
    crate::hud::mounted::receive(
        &state,
        &Event::Slave(SlaveEvent::RequestRejected {
            command: SlaveCommand::Bind { tl_id: 8 },
            message: "slave timeline ID must be nonzero".into(),
        }),
    );
    assert!(!state.borrow_mut().mounted.bind_pending(Instant::now()));
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
}

#[test]
fn a_server_bind_refusal_clears_the_pending_request() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
    // SLAVE_CANNOT_BIND: the server's SCErrorMsg answer to CSBindSlave.
    crate::hud::mounted::receive(&state, &Event::ServerError { code: 0x60 });
    assert!(!state.borrow_mut().mounted.bind_pending(Instant::now()));
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
}

#[test]
fn an_unanswered_bind_expires_after_the_safety_timeout() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
    let now = Instant::now() + crate::hud::mounted::BIND_TIMEOUT;
    assert!(!state.borrow_mut().mounted.bind_pending(now));
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
}

#[test]
fn player_death_clears_the_pending_bind_and_refuses_the_next_one() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
    crate::hud::mounted::receive(&state, &Event::UnitDeath { obj_id: 100 });
    assert!(!state.borrow_mut().mounted.bind_pending(Instant::now()));
    assert_eq!(bind(&state, &units, 20), Outcome::Refused(0x14c));
    crate::hud::mounted::receive(
        &state,
        &Event::Resurrected {
            obj_id: 100,
            pos: Pos::default(),
            yaw: 0.0,
        },
    );
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
}

#[test]
fn detaching_clears_the_pending_bind() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
    crate::hud::mounted::receive(
        &state,
        &Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
    );
    assert!(!state.borrow_mut().mounted.bind_pending(Instant::now()));
    assert_eq!(bind(&state, &units, 20), Outcome::Sent);
}

#[test]
fn a_dead_player_cannot_bind() {
    let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    crate::hud::mounted::receive(&state, &Event::UnitDeath { obj_id: 100 });
    assert_eq!(bind(&state, &units, 20), Outcome::Refused(0x14c));
    assert!(state.borrow().requests.is_empty());
}

#[test]
fn despawning_escaping_or_losing_the_bound_slave_clears_it() {
    for event in [
        Event::Slave(SlaveEvent::Despawn { obj_id: 20 }),
        Event::Slave(SlaveEvent::Escaped(aa_net::SlaveEscape {
            obj_id: 20,
            x: 0,
            y: 0,
            z: 0.0,
            yaw: 0.0,
        })),
        Event::Slave(SlaveEvent::Removed {
            obj_id: 20,
            tl_id: 8,
        }),
        Event::UnitsRemoved(vec![20]),
    ] {
        let (state, units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
        bound(&state, 20);
        assert!(state.borrow().mounted.bound_slave().is_some());
        crate::hud::mounted::receive(&state, &event);
        assert!(
            state.borrow().mounted.bound_slave().is_none(),
            "{event:?} leaves no stale bound slave"
        );
        assert_eq!(bind(&state, &units, 20), Outcome::Sent);
    }
}

#[test]
fn the_driven_cart_uses_the_controller_speed() {
    let (state, _units) = setup(&[slave(20, 8, Vec3::new(1.0, 0.0, 0.0))]);
    bound(&state, 20);
    // no UnitMoved reports the cart this client drives
    state.borrow_mut().mounted.local_cart = Some((20, 5.5));
    assert_eq!(discard(&state), Outcome::Refused(0x67));
    state.borrow_mut().mounted.local_cart = Some((20, -4.0));
    assert_eq!(discard(&state), Outcome::Sent);
}
