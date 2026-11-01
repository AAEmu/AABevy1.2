use super::*;
use aa_net::Event;
use aa_net::buff::Buff;
use bevy::prelude::*;
use std::{cell::RefCell, rc::Rc};

fn game() -> crate::hud::data::GameData {
    let mut game = crate::hud::data::GameData::default();
    for (id, skills, kind) in [
        (5068, (20848..=20855).collect(), BuffKind::Good),
        (6010, vec![22213, 22214, 22217], BuffKind::Hidden),
        (8079, vec![], BuffKind::Good),
        (1, vec![71], BuffKind::Good),
        (2, vec![72], BuffKind::Bad),
        (3, vec![73], BuffKind::Hidden),
    ] {
        game.mode_actions.skills.insert(id, skills);
        game.native_buffs.insert(
            id,
            aa_net::state::NativeBuffRule {
                kind,
                per_unit_creation: false,
                stack_rule_id: 6,
                max_stack: 1,
                max_charge: 0,
            },
        );
    }
    // the Farm Wagon's slave template (60) as `slave_mount_skills` holds it
    game.mode_actions.slave_skills.insert(
        60,
        vec![
            SlaveSkill::new(20704, 201),
            SlaveSkill::new(20291, 190),
            SlaveSkill::new(15622, 65),
            SlaveSkill::new(15450, 56),
            SlaveSkill::new(15546, 59),
        ],
    );
    // slave template 73 (the cotton-field wagon) has its own two rows
    game.mode_actions.slave_skills.insert(
        73,
        vec![SlaveSkill::new(15622, 65), SlaveSkill::new(17000, 75)],
    );
    game
}

/// The buff/player skills of the shown bar (entries in order).
fn players(entries: &[Entry]) -> Vec<u32> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Player(skill) => Some(*skill),
            _ => None,
        })
        .collect()
}

/// Every skill id of the shown bar: the slave's and the player's, functions dropped.
fn skill_ids(entries: &[Entry]) -> Vec<u32> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Player(skill) => Some(*skill),
            Entry::Skill(skill) => Some(skill.skill),
            Entry::Function(_) => None,
        })
        .collect()
}

fn functions(entries: &[Entry]) -> Vec<Function> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Function(function) => Some(*function),
            _ => None,
        })
        .collect()
}

/// A cannon's gunner bar: the point-1 attached skill is shown (14372), the slot casts the linked
/// mount skill (10473, `target_type_id` 11). The two extra skills make the gate visible.
fn cannon_game() -> crate::hud::data::GameData {
    let mut game = game();
    game.skills.insert(
        14372,
        crate::hud::data::SkillInfo {
            target_type: 0,
            ..default()
        },
    );
    game.skills.insert(
        10473,
        crate::hud::data::SkillInfo {
            target_type: 11,
            ..default()
        },
    );
    game.mode_actions
        .slave_skills
        .insert(10, vec![SlaveSkill::new(10473, 16)]);
    game.mode_actions.attached.insert((10, 16, 1), 14372);
    game.mode_actions.first_attached.insert((10, 16), 1);
    game
}

fn cannon_unit(id: u32, template_id: u32) -> aa_net::model::Unit {
    let mut unit = slave_unit(id, template_id);
    unit.kind = aa_net::model::UnitKind::Slave {
        id: 8,
        tl_id: 75,
        template_id,
        summoner_id: 100,
    };
    unit.model_id = 117;
    unit
}

/// Press one driver-bar slot with a fresh world and return the commands it wrote.
fn press_driver_slot(state: &Shared, slot: usize) -> Vec<aa_net::Command> {
    use crate::hud::{
        alerts::UseFailed,
        usable::{Env, InFlight},
    };
    use crate::ingame::ground_target::SkillPressed;
    use bevy::ecs::system::SystemState;
    let mut world = World::new();
    world.init_resource::<Messages<SkillPressed>>();
    world.init_resource::<Messages<aa_net::NetCommand>>();
    world.init_resource::<Messages<UseFailed>>();
    let mut writers: SystemState<(
        MessageWriter<SkillPressed>,
        MessageWriter<aa_net::NetCommand>,
        MessageWriter<UseFailed>,
    )> = SystemState::new(&mut world);
    let (mut pressed, mut cast, mut failed) = writers.get_mut(&mut world).unwrap();
    let data = state.borrow();
    let env = Env::of(&data, None, &[], None);
    let mut target = aa_net::state::Target::default();
    target.obj_id = Some(777);
    use_slot(
        &data,
        slot,
        &env,
        &mut InFlight::default(),
        &target,
        &mut pressed,
        &mut cast,
        &mut failed,
    );
    drop(data);
    world
        .resource_mut::<Messages<aa_net::NetCommand>>()
        .drain()
        .map(|command| command.0)
        .collect()
}

fn runtime(loaded: bool) -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let rt = UiRuntime::new("en_us", 1280., 720.)?;
    rt.lua.load("HUD={};X2Unit={};OBS={}").exec()?;
    let state = Rc::new(RefCell::new(HudData::default()));
    if loaded {
        state.borrow().game.0.set(game()).ok().unwrap();
    }
    install(&rt.lua, &state)?;
    rt.lua
        .load(
            r#"
      local w=UIParent:CreateWidget('emptywidget','modeObserver','UIParent')
      w:RegisterEvent('BUFF_SKILL_CHANGED');w:RegisterEvent('MODE_ACTIONS_UPDATE')
      w:SetHandler('OnEvent',function(_,event)
        OBS[#OBS+1]={event,X2Unit:GetModeActionsCount()}
        if REENTER then REENTER(event) end
      end)
    "#,
        )
        .exec()?;
    crate::hud::user_music::receipt(
        None,
        &state,
        &Event::ReadyToConnect {
            world_id: 17,
            cookie: aa_net::login::WorldCookie {
                cookie: 0,
                addr: "127.0.0.1:1".parse()?,
            },
        },
    );
    Ok((rt, state))
}

fn spawn(state: &Shared, rt: Option<&UiRuntime>, buffs: Vec<Buff>) {
    let mut spawn = crate::hud::expansion::fixture::spawn();
    spawn.unit.buffs = buffs;
    crate::hud::user_music::receipt(rt, state, &Event::PlayerSpawn(Box::new(spawn)));
}

fn buff(id: u32, index: u32, kind: BuffKind) -> Buff {
    Buff {
        buff_id: id,
        index,
        kind,
        ..default()
    }
}
fn created(id: u32, index: u32) -> Event {
    Event::BuffCreated {
        obj_id: 100,
        buff: buff(id, index, BuffKind::Good),
    }
}
fn receipt(rt: &UiRuntime, state: &Shared, event: Event) {
    crate::hud::user_music::receipt(Some(rt), state, &event);
}

#[test]
fn native_sql_order_duplicates_and_missing_descriptors_are_preserved()
-> Result<(), Box<dyn std::error::Error>> {
    let Ok(db) = Connection::open_with_flags(
        std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string()),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        eprintln!("skip: no client database");
        return Ok(());
    };
    let tables = Tables::load(&db)?;
    assert_eq!(tables.skills.values().map(Vec::len).sum::<usize>(), 546);
    assert_eq!(
        tables.skills[&4438],
        [17657, 19167, 19167, 13435, 13440, 14236]
    );
    assert_eq!(tables.skills[&3530], [13435, 13440, 14236, 17657]);
    assert_eq!(tables.skills[&5068], (20848..=20855).collect::<Vec<_>>());
    assert_eq!(tables.skills[&6010], [22213, 22214, 22217]);
    assert!(!tables.has(8079));
    Ok(())
}

#[test]
fn the_farm_wagon_slave_bar_comes_from_slave_mount_skills() -> Result<(), Box<dyn std::error::Error>>
{
    let Ok(db) = Connection::open_with_flags(
        std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string()),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        eprintln!("skip: no client database");
        return Ok(());
    };
    let tables = Tables::load(&db)?;
    let skills = &tables.slave_skills[&60];
    assert_eq!(
        skills.iter().map(|s| s.skill).collect::<Vec<_>>(),
        [20704, 20291, 15622, 15450, 15546],
        "the capture's slots 1..5, in row order"
    );
    assert_eq!(
        skills.iter().map(|s| s.mount_skill).collect::<Vec<_>>(),
        [201, 190, 65, 56, 59]
    );
    // the bar's cooldowns and cast time are the skill data the capture shows (10 s / 5 s)
    let cooldown: i64 = db.query_row(
        "SELECT cooldown_time FROM skills WHERE id = 20291",
        [],
        |r| r.get(0),
    )?;
    let casting: i64 = db.query_row(
        "SELECT casting_time FROM skills WHERE id = 15546",
        [],
        |r| r.get(0),
    )?;
    assert_eq!((cooldown, casting), (10_000, 5_000));
    assert!(
        !tables.has_passenger_point(60),
        "the Farm Wagon model 1008 has no point 2"
    );
    assert!(
        tables.has_passenger_point(146),
        "the Red Farm Hauler has point 2"
    );
    // No Farm Wagon row has an attached list, so all five count at point 1 and none at point 2.
    assert_eq!(
        tables
            .bar(60, 1, false)
            .expect("Farm Wagon driver bar")
            .len(),
        5
    );
    assert!(
        tables.bar(60, 2, false).unwrap_or_default().is_empty(),
        "the Farm Wagon has no point-2 seat skill"
    );
    assert_eq!(
        tables.text(Function::Unbind),
        "Disembark from current device."
    );
    assert_eq!(
        tables.text(Function::ShowSlaveInfo),
        "Check details on a summon."
    );
    assert_eq!(tables.text(Function::CommandUnmount), "Dismount");
    Ok(())
}

/// `FUN_39347260` filters every seat, the driver's included: the tank 46's rows are 201 (no
/// attached list), 45 (first `mount_attached_skills` entry point 1) and 51 (first entry point 2).
/// The native driver bar is therefore 201 and 45; only the point-2 seat lists 51. The old port
/// kept every row for point 1, so the passenger skill 20238 appeared on the driver bar too.
#[test]
fn the_real_tank_46_bars_follow_the_seat_point() {
    let Some(conn) = Connection::open_with_flags(
        std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string()),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok() else {
        eprintln!("skip: no client database");
        return;
    };
    let tables = Tables::load(&conn).expect("mode action tables");
    let driver = tables.bar(46, 1, false).expect("tank 46 driver bar");
    assert_eq!(
        driver
            .iter()
            .map(|s| (s.skill, s.mount_skill, s.fire))
            .collect::<Vec<_>>(),
        [(14373, 45, None), (20704, 201, None)],
        "the driver keeps only the rows whose first attached point is 1 or absent"
    );
    let passenger = tables.bar(46, 2, false).expect("tank 46 passenger bar");
    assert_eq!(
        passenger
            .iter()
            .map(|s| (s.skill, s.mount_skill, s.cast_skill()))
            .collect::<Vec<_>>(),
        [(20238, 51, 15235)],
        "the point-2 seat shows 20238 and casts 15235"
    );
}

fn slave_unit(id: u32, template_id: u32) -> aa_net::model::Unit {
    let mut unit = crate::hud::expansion::fixture::spawn().unit;
    unit.obj_id = id;
    unit.kind = aa_net::model::UnitKind::Slave {
        id: 700,
        tl_id: 75,
        template_id,
        summoner_id: 100,
    };
    unit
}

#[test]
fn a_slave_driver_bar_replaces_the_buff_skills_and_leaving_restores_them()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(true)?;
    // through the mounted view too, which the bare `spawn` helper does not feed
    let mut own = crate::hud::expansion::fixture::spawn();
    own.unit.buffs = vec![buff(5068, 9, BuffKind::Good)];
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    assert_eq!(actions(&state.borrow()).len(), 8);
    // a passenger seat gets the slave's bar: 1.2 appends `unbind` for any seat
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitSpawned(Box::new(slave_unit(20, 60))),
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 2,
            reason: 0,
        },
        None,
    );
    assert_eq!(actions(&state.borrow()).len(), 1);
    assert_eq!(
        functions(actions(&state.borrow())),
        [Function::Unbind],
        "a passenger gets only the unbind function slot"
    );
    // the driver's seat switches to the slave's skills (the native window also appends two function
    // buttons, slots 6 and 7: not skills, not part of this list)
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    assert_eq!(
        state.borrow().mounted.driver_slave(),
        Some((20, 60)),
        "the mounted view sees the driver seat"
    );
    let driver = state.borrow();
    assert_eq!(
        skill_ids(actions(&driver)),
        [20704, 20291, 15622, 15450, 15546],
        "the capture's five skill slots"
    );
    assert_eq!(
        functions(actions(&driver)),
        [Function::Unbind, Function::ShowSlaveInfo],
        "slots 6 and 7"
    );
    assert_eq!(super::function_at(&driver, 5), Some(Function::Unbind));
    assert_eq!(
        super::function_at(&driver, 6),
        Some(Function::ShowSlaveInfo)
    );
    assert_eq!(super::function_at(&driver, 7), None);
    assert_eq!(super::function_at(&driver, 0), None);
    drop(driver);
    assert_eq!(
        rt.lua
            .load("return X2Unit:GetModeActionsCount()")
            .eval::<usize>()?,
        7
    );
    // leaving the driver's seat returns to the buff skills
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
        None,
    );
    assert_eq!(actions(&state.borrow()).len(), 8);
    // another slave takes its own rows, in that slave's order
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitSpawned(Box::new(slave_unit(30, 73))),
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 30,
            point: 1,
            reason: 1,
        },
        None,
    );
    assert_eq!(skill_ids(actions(&state.borrow())), [15622, 17000]);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
        None,
    );
    assert_eq!(actions(&state.borrow()).len(), 8);
    // the slave despawns while the player is on its driver seat: the previous bar comes back once
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    assert_eq!(
        skill_ids(actions(&state.borrow())),
        [20704, 20291, 15622, 15450, 15546]
    );
    rt.lua.load("SAVED=#OBS").exec()?;
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::UnitsRemoved(vec![20]), None);
    assert_eq!(actions(&state.borrow()).len(), 8);
    rt.lua.load("assert(#OBS==SAVED+2)").exec()?;
    // an unrelated removal does not publish the restored bar again
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::UnitsRemoved(vec![999]), None);
    rt.lua.load("assert(#OBS==SAVED+2)").exec()?;
    Ok(())
}

/// 39 of the 140 slaves have no `slave_mount_skills` rows (the carriage 7, the rowboats 55/91 and
/// the ship 23 among them). `FUN_39204500` appends `unbind` after the skill loop whether or not it
/// produced anything, and the driver still gets `showSlaveInfo` (DEC 404388-404393).
#[test]
fn a_slave_with_no_mount_skills_keeps_the_function_slots() -> Result<(), Box<dyn std::error::Error>>
{
    let (rt, state) = runtime(true)?;
    let mut own = crate::hud::expansion::fixture::spawn();
    own.unit.buffs = vec![buff(5068, 9, BuffKind::Good)];
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitSpawned(Box::new(slave_unit(20, 55))),
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    let bar = state.borrow();
    assert!(
        skill_ids(actions(&bar)).is_empty(),
        "the rowboat has no mount skills"
    );
    assert_eq!(
        functions(actions(&bar)),
        [Function::Unbind, Function::ShowSlaveInfo],
        "the driver keeps slot 6 and slot 7"
    );
    assert_eq!(actions(&bar).len(), 2);
    drop(bar);
    assert_eq!(
        rt.lua
            .load("return X2Unit:GetModeActionsCount()")
            .eval::<usize>()?,
        2
    );
    // a passenger of the same slave: `unbind` only (the native appends it on any seat)
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 2,
            reason: 0,
        },
        None,
    );
    assert_eq!(skill_ids(actions(&state.borrow())).len(), 0);
    assert_eq!(functions(actions(&state.borrow())), [Function::Unbind]);
    Ok(())
}

/// A point-2 seat lists the slave's mount skills attached to that point (`FUN_39347260` with the
/// seat point), before `unbind`. The tank 46's row mount skill 51 has `mount_attached_skills`
/// 20238 at point 2 and casts its `mount_skills.skill_id` 15235, like a gunner row.
#[test]
fn a_passenger_bar_lists_the_point_2_attached_skills_before_unbind()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(false)?;
    {
        let data = state.borrow_mut();
        let mut game = game();
        game.mode_actions
            .slave_skills
            .insert(46, vec![SlaveSkill::new(15235, 51)]);
        game.mode_actions.attached.insert((46, 51, 2), 20238);
        game.mode_actions.first_attached.insert((46, 51), 2);
        data.game.0.set(game).ok().unwrap();
    }
    let mut own = crate::hud::expansion::fixture::spawn();
    own.unit.buffs = vec![buff(5068, 9, BuffKind::Good)];
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitSpawned(Box::new(slave_unit(20, 46))),
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 2,
            reason: 0,
        },
        None,
    );
    let bar = state.borrow();
    assert_eq!(skill_ids(actions(&bar)), [20238], "the point-2 row");
    let Entry::Skill(skill) = actions(&bar)[0] else {
        panic!("a skill entry");
    };
    assert_eq!((skill.mount_skill, skill.cast_skill()), (51, 15235));
    assert_eq!(functions(actions(&bar)), [Function::Unbind]);
    drop(bar);
    // the slot casts as the ridden slave, not as the player
    let sent = press_driver_slot(&state, 0);
    assert!(matches!(
        sent.as_slice(),
        [aa_net::Command::UseMateSkill {
            skill_id: 15235,
            mate_obj_id: 20,
            mount_skill_id: 51,
            ..
        }]
    ));
    Ok(())
}

/// A press on the driven slave's bar starts the slave's skill as a `Mount` caster: the slave's object id
/// and the `mount_skills` row, never the player's unit (the server only takes slave skills from a mount caster;
/// `skills/handlers.rs` reads `SkillCaster::Mount`). The captured `CSStartSkill` of a mate's bar has the same shape.
#[test]
fn a_driver_bar_slot_casts_the_slave_skill_with_the_mount_caster()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::hud::{
        alerts::UseFailed,
        usable::{Env, InFlight},
    };
    use crate::ingame::ground_target::SkillPressed;
    use bevy::ecs::system::SystemState;
    let (rt, state) = runtime(true)?;
    let mut own = crate::hud::expansion::fixture::spawn();
    own.unit.buffs = vec![buff(5068, 9, BuffKind::Good)];
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitSpawned(Box::new(slave_unit(20, 60))),
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    assert_eq!(
        skill_ids(actions(&state.borrow())),
        [20704, 20291, 15622, 15450, 15546]
    );

    let mut world = World::new();
    world.init_resource::<Messages<SkillPressed>>();
    world.init_resource::<Messages<aa_net::NetCommand>>();
    world.init_resource::<Messages<UseFailed>>();
    let mut writers: SystemState<(
        MessageWriter<SkillPressed>,
        MessageWriter<aa_net::NetCommand>,
        MessageWriter<UseFailed>,
    )> = SystemState::new(&mut world);
    let (mut pressed, mut cast, mut failed) = writers.get_mut(&mut world).unwrap();
    let data = state.borrow();
    let env = Env::of(&data, None, &[], None);
    let mut target = aa_net::state::Target::default();
    target.obj_id = Some(777);
    use_slot(
        &data,
        0,
        &env,
        &mut InFlight::default(),
        &target,
        &mut pressed,
        &mut cast,
        &mut failed,
    );
    drop(data);
    assert!(
        world
            .resource_mut::<Messages<SkillPressed>>()
            .drain()
            .next()
            .is_none(),
        "a slave slot is not a player cast"
    );
    assert!(
        world
            .resource_mut::<Messages<UseFailed>>()
            .drain()
            .next()
            .is_none()
    );
    let sent: Vec<_> = world
        .resource_mut::<Messages<aa_net::NetCommand>>()
        .drain()
        .map(|command| command.0)
        .collect();
    let [command] = &sent[..] else {
        panic!("{sent:?}")
    };
    let aa_net::Command::UseMateSkill {
        skill_id,
        mate_obj_id,
        mount_skill_id,
        target_obj_id,
    } = command
    else {
        panic!("{command:?}")
    };
    assert_eq!(
        (*skill_id, *mate_obj_id, *mount_skill_id, *target_obj_id),
        (20704, 20, 201, 777)
    );
    let body = command
        .mate_skill_message(100)
        .expect("the mount cast")
        .encode_body();
    assert_eq!(
        body,
        [
            0xE0, 0x50, 0, 0, 3, 0x14, 0, 0, 0xC9, 0, 0, 0, 0, 0x09, 0x03, 0, 0
        ]
    );
    Ok(())
}

/// A schema fixture mirroring the real rows: cotton cart 73 (kind 6, velocity 4) and galleon
/// cannon 10 (kind 8, velocity 0), each with its `slave_mount_skills` and point-1
/// `mount_attached_skills` rows.
fn schema_game() -> crate::hud::data::GameData {
    let conn = Connection::open_in_memory().expect("in-memory schema");
    conn.execute_batch(
        "CREATE TABLE buffs(id INTEGER);
         CREATE TABLE skills(id INTEGER);
         CREATE TABLE buff_skills(buff_id INTEGER, skill_id INTEGER);
         CREATE TABLE slave_mount_skills(id INTEGER, slave_id INTEGER, mount_skill_id INTEGER);
         CREATE TABLE mount_skills(id INTEGER, skill_id INTEGER);
         CREATE TABLE mount_attached_skills(id INTEGER, mount_skill_id INTEGER, attach_point_id INTEGER, skill_id INTEGER);
         CREATE TABLE slaves(id INTEGER, name TEXT, slave_kind_id INTEGER, model_id INTEGER);
         CREATE TABLE models(id INTEGER, sub_id INTEGER, sub_type TEXT);
         CREATE TABLE vehicle_models(id INTEGER, velocity REAL,
             turret_pitch_angle_min REAL, turret_pitch_angle_max REAL,
             turret_yaw_angle_min REAL, turret_yaw_angle_max REAL,
             turret_yaw_angvel REAL, turret_pitch_angvel REAL);
         INSERT INTO mount_skills VALUES (65, 15622), (75, 17000), (16, 10473);
         INSERT INTO slave_mount_skills VALUES (56, 73, 65), (57, 73, 75), (25, 10, 16);
         INSERT INTO mount_attached_skills VALUES (1, 65, 1, 15548), (2, 16, 1, 14372);
         INSERT INTO slaves VALUES (73, 'Cotton', 7, 1008), (10, 'Cannon', 8, 117);
         INSERT INTO models VALUES (1008, 38, 'VehicleModel'), (117, 10, 'VehicleModel');
         INSERT INTO vehicle_models VALUES
             (38, 4.0, -60, 60, -360, 360, 30, 10),
             (10, 0.0, -20, 35, -22, 22, 35, 20);",
    )
    .expect("fixture rows");
    let tables = Tables::load(&conn).expect("fixture tables");
    let mut game = crate::hud::data::GameData::default();
    game.mode_actions = tables;
    game
}

fn schema_unit(id: u32, kind: u32, template: u32, model: u32) -> aa_net::model::Unit {
    let mut unit = slave_unit(id, template);
    unit.kind = aa_net::model::UnitKind::Slave {
        id: kind,
        tl_id: 75,
        template_id: template,
        summoner_id: 100,
    };
    unit.model_id = model;
    unit
}

fn bar_skills(state: &Shared) -> Vec<(u32, u32)> {
    seat_bar(&state.borrow())
        .expect("driver bar")
        .iter()
        .filter_map(|entry| match entry {
            Entry::Skill(skill) => Some((skill.skill, skill.cast_skill())),
            _ => None,
        })
        .collect()
}

/// `seat_bar` on the same rows the real data has: the cart 73 keeps its mount skills, the
/// cannon 10 shows the attached skill when the engine's seat flag names it or, before that flag
/// arrives, when the template gate says it is a turret. A stale flag for another slave also
/// falls back to the template gate.
#[test]
fn seat_bar_uses_the_engine_seat_or_the_template_fallback() -> Result<(), Box<dyn std::error::Error>>
{
    if std::env::var("AA_PAK").is_err() {
        eprintln!("skip: no client pak");
        return Ok(());
    }
    let (rt, state) = runtime(false)?;
    state.borrow().game.0.set(schema_game()).ok().unwrap();
    let own = crate::hud::expansion::fixture::spawn();
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    let cart = schema_unit(30, 7, 73, 1008);
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::UnitSpawned(Box::new(cart)), None);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 30,
            point: 1,
            reason: 1,
        },
        None,
    );
    assert_eq!(bar_skills(&state), [(15622, 15622), (17000, 17000)]);
    assert!(
        !state
            .borrow()
            .game
            .get()
            .unwrap()
            .mode_actions
            .turret_slave(73),
        "the velocity-4 cart is not in the template turret set"
    );

    let cannon = schema_unit(20, 8, 10, 117);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitSpawned(Box::new(cannon)),
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    assert!(
        state
            .borrow()
            .game
            .get()
            .unwrap()
            .mode_actions
            .turret_slave(10),
        "the cannon is in the template turret set"
    );
    assert_eq!(bar_skills(&state), [(14372, 10473)], "template fallback");
    state.borrow_mut().turret_seat = Some(20);
    assert_eq!(bar_skills(&state), [(14372, 10473)], "engine seat flag");
    state.borrow_mut().turret_seat = Some(30);
    assert_eq!(
        bar_skills(&state),
        [(14372, 10473)],
        "stale flag, template fallback"
    );
    Ok(())
}

/// A cannon gunner's bar shows the point-1 attached skill (14372) but casts the linked mount
/// skill (10473). With a published sight that BallisticPos cast goes to the aimed spot; without
/// one it keeps the ordinary unit target. This is the real 1.2 split: the bar skill has
/// `target_type_id` 0, the mount skill it casts has 11.
#[test]
fn a_manned_cannon_slot_fires_at_the_aimed_spot() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(false)?;
    state.borrow().game.0.set(cannon_game()).ok().unwrap();
    let mut own = crate::hud::expansion::fixture::spawn();
    own.unit.buffs = vec![buff(5068, 9, BuffKind::Good)];
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitSpawned(Box::new(cannon_unit(20, 10))),
        None,
    );
    // The engine seat flag is what the real attach path has by the time the HUD rebuilds the
    // bar; `cannon_game` does not put slave 10 in the template fallback set.
    state.borrow_mut().turret_seat = Some(20);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    assert_eq!(
        skill_ids(actions(&state.borrow())),
        [14372],
        "the real gunner bar skill"
    );
    state.borrow_mut().turret = Some(crate::ingame::turret::Sight {
        slave: 20,
        aim: aa_net::TurretAim {
            pitch: 0.3,
            yaw: -0.2,
        },
        pos: aa_net::Pos::new(130.25, 131.5, 100.0),
        rot: 0.7,
    });
    let sent = press_driver_slot(&state, 0);
    let [
        aa_net::Command::UseMateSkillAt {
            skill_id,
            mate_obj_id,
            mount_skill_id,
            pos,
            rot,
        },
    ] = &sent[..]
    else {
        panic!("{sent:?}")
    };
    assert_eq!((*skill_id, *mate_obj_id, *mount_skill_id), (10473, 20, 16));
    assert_eq!(*pos, aa_net::Pos::new(130.25, 131.5, 100.0));
    assert_eq!(*rot, 0.7);

    // Not manning the cannon (no sight): the same cast keeps the selected unit.
    state.borrow_mut().turret = None;
    let sent = press_driver_slot(&state, 0);
    let [
        aa_net::Command::UseMateSkill {
            skill_id,
            mate_obj_id,
            mount_skill_id,
            target_obj_id,
        },
    ] = &sent[..]
    else {
        panic!("{sent:?}")
    };
    assert_eq!(
        (*skill_id, *mate_obj_id, *mount_skill_id, *target_obj_id),
        (10473, 20, 16, 777)
    );
    Ok(())
}

/// The player on the Farm Wagon's driver seat with a world whose `Skills` is the net state.
fn driver_setup() -> Result<(UiRuntime, Shared, World), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(false)?;
    {
        let mut game = game();
        game.skills.insert(
            20291,
            crate::hud::data::SkillInfo {
                cooldown_ms: 10_000,
                gcd_ms: 5_000,
                ..default()
            },
        );
        state.borrow().game.0.set(game).ok().unwrap();
    }
    let mut own = crate::hud::expansion::fixture::spawn();
    own.unit.buffs = vec![buff(5068, 9, BuffKind::Good)];
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitSpawned(Box::new(slave_unit(20, 60))),
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    assert_eq!(
        skill_ids(actions(&state.borrow())),
        [20704, 20291, 15622, 15450, 15546]
    );
    let mut world = World::new();
    world.insert_resource(aa_net::state::Skills {
        obj_id: 100,
        ..default()
    });
    world.init_resource::<Messages<crate::ingame::ground_target::SkillPressed>>();
    world.init_resource::<Messages<aa_net::NetCommand>>();
    world.init_resource::<Messages<crate::hud::alerts::UseFailed>>();
    Ok((rt, state, world))
}

/// The server's accepted slave cast: `SCSkillStarted` with the slave as a `Mount` caster.
fn mount_started(caster: u32, skill: u32, tl: u16, cast_ms: u32) -> Event {
    Event::SkillStarted(aa_net::skill::SkillStarted {
        skill_id: skill,
        tl,
        caster: aa_net::buff::BuffCaster {
            kind: aa_net::buff::caster_kind::MOUNT,
            obj_id: caster,
        },
        target: aa_net::skill::SkillTarget::unit(777),
        raw_caster: aa_net::SkillCasterData::Mount {
            obj_id: caster,
            mount_skill_id: 0,
        },
        raw_target: aa_net::SkillTargetData::Unit(777),
        cast_ms,
        base_cast_ms: cast_ms,
        synergy: 0,
        result: 0,
        result_value: 0,
        raw_result: Default::default(),
    })
}

/// The server's instant slave cast: `SCSkillFired` with the slave as a `Mount` caster.
fn mount_fired(caster: u32, skill: u32) -> Event {
    Event::SkillFired(aa_net::skill::SkillFired {
        skill_id: skill,
        tl: 9,
        caster: aa_net::buff::BuffCaster {
            kind: aa_net::buff::caster_kind::MOUNT,
            obj_id: caster,
        },
        target: aa_net::skill::SkillTarget::unit(777),
        raw_caster: aa_net::SkillCasterData::Mount {
            obj_id: caster,
            mount_skill_id: 0,
        },
        raw_target: aa_net::SkillTargetData::Unit(777),
        delay_ms: 0,
        channeling_ms: 0,
        fire_anim_id: 0,
        result: Default::default(),
        raw_final_flag: 0,
    })
}

/// The driven slave's own timer, read from its `Mount` receipts, drives the slot and the local
/// refusal; the player's `Skills` never sees it, and stepping off and back on keeps it running.
#[test]
fn a_slave_cooldown_shows_refuses_and_survives_reboarding() -> Result<(), Box<dyn std::error::Error>>
{
    use crate::hud::{
        alerts::UseFailed,
        usable::{Env, InFlight},
    };
    use crate::ingame::ground_target::SkillPressed;
    use bevy::ecs::system::SystemState;
    let (rt, state, mut world) = driver_setup()?;
    world
        .resource_mut::<aa_net::state::Skills>()
        .apply(&mount_fired(20, 20291));
    refresh_slots(&world, &mut state.borrow_mut());
    {
        let data = state.borrow();
        let slot = &data.mode_actions.slots[1];
        assert_eq!(slot.id, 20291);
        assert_eq!(
            slot.cooldown.map(|(_, total)| total),
            Some(10_000),
            "the slot shows the slave's 10 s cooldown"
        );
    }
    let skills = world.resource::<aa_net::state::Skills>().clone();
    assert!(skills.last_used.is_empty() && skills.last_cast.is_none());
    // A press during it is refused locally: nothing goes to the server.
    {
        let mut writers: SystemState<(
            MessageWriter<SkillPressed>,
            MessageWriter<aa_net::NetCommand>,
            MessageWriter<UseFailed>,
        )> = SystemState::new(&mut world);
        let (mut pressed, mut cast, mut fail) = writers.get_mut(&mut world).unwrap();
        let data = state.borrow();
        let env = Env::of(&data, Some(&skills), &[], None);
        let mut target = aa_net::state::Target::default();
        target.obj_id = Some(777);
        use_slot(
            &data,
            1,
            &env,
            &mut InFlight::default(),
            &target,
            &mut pressed,
            &mut cast,
            &mut fail,
        );
    }
    assert!(
        world
            .resource_mut::<Messages<aa_net::NetCommand>>()
            .drain()
            .next()
            .is_none()
    );
    let failed: Vec<_> = world
        .resource_mut::<Messages<UseFailed>>()
        .drain()
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].result, crate::hud::usable::code::COOLDOWN_TIME);
    // Leaving and re-boarding the same slave keeps its running timer.
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    refresh_slots(&world, &mut state.borrow_mut());
    assert!(
        state.borrow().mode_actions.slots[1].cooldown.is_some(),
        "the slave's timer survives re-boarding"
    );
    // Ten seconds later the slot is ready and the press goes out as the slave's cast.
    {
        let at = web_time::Instant::now() - std::time::Duration::from_secs(10);
        let mut skills = world.resource_mut::<aa_net::state::Skills>();
        skills.mount_last_used.insert((20, 20291), at);
        skills.mount_last_cast.insert(20, (at, 20291));
    }
    refresh_slots(&world, &mut state.borrow_mut());
    assert!(state.borrow().mode_actions.slots[1].cooldown.is_none());
    let skills = world.resource::<aa_net::state::Skills>().clone();
    {
        let mut writers: SystemState<(
            MessageWriter<SkillPressed>,
            MessageWriter<aa_net::NetCommand>,
            MessageWriter<UseFailed>,
        )> = SystemState::new(&mut world);
        let (mut pressed, mut cast, mut fail) = writers.get_mut(&mut world).unwrap();
        let data = state.borrow();
        let env = Env::of(&data, Some(&skills), &[], None);
        let mut target = aa_net::state::Target::default();
        target.obj_id = Some(777);
        use_slot(
            &data,
            1,
            &env,
            &mut InFlight::default(),
            &target,
            &mut pressed,
            &mut cast,
            &mut fail,
        );
    }
    assert!(
        world
            .resource_mut::<Messages<UseFailed>>()
            .drain()
            .next()
            .is_none()
    );
    let sent: Vec<_> = world
        .resource_mut::<Messages<aa_net::NetCommand>>()
        .drain()
        .map(|command| command.0)
        .collect();
    assert!(
        matches!(
            &sent[..],
            [aa_net::Command::UseMateSkill {
                skill_id: 20291,
                mate_obj_id: 20,
                ..
            }]
        ),
        "{sent:?}"
    );
    Ok(())
}

/// A driver slot is the slave's skill: the player's own cooldown and global cooldown must not refuse it.
#[test]
fn a_driver_slot_ignores_the_players_cooldown_and_gcd() -> Result<(), Box<dyn std::error::Error>> {
    use crate::hud::{
        alerts::UseFailed,
        usable::{Env, InFlight},
    };
    use crate::ingame::ground_target::SkillPressed;
    use bevy::ecs::system::SystemState;
    let (_rt, state, mut world) = driver_setup()?;
    let now = web_time::Instant::now();
    let mut skills = aa_net::state::Skills {
        obj_id: 100,
        last_cast: Some(now),
        last_cast_skill: 20291,
        ..default()
    };
    skills.last_used.insert(20291, now);
    {
        let data = state.borrow();
        let env = Env::of(&data, Some(&skills), &[], None);
        assert!(
            env.held_back(20291, &InFlight::default()),
            "the player's global cooldown would hold the press"
        );
        assert!(
            env.precheck(20291).is_some(),
            "the player's own cooldown would refuse it"
        );
    }
    {
        let mut writers: SystemState<(
            MessageWriter<SkillPressed>,
            MessageWriter<aa_net::NetCommand>,
            MessageWriter<UseFailed>,
        )> = SystemState::new(&mut world);
        let (mut pressed, mut cast, mut fail) = writers.get_mut(&mut world).unwrap();
        let data = state.borrow();
        let env = Env::of(&data, Some(&skills), &[], None);
        let mut target = aa_net::state::Target::default();
        target.obj_id = Some(777);
        use_slot(
            &data,
            1,
            &env,
            &mut InFlight::default(),
            &target,
            &mut pressed,
            &mut cast,
            &mut fail,
        );
    }
    assert!(
        world
            .resource_mut::<Messages<UseFailed>>()
            .drain()
            .next()
            .is_none()
    );
    let sent: Vec<_> = world
        .resource_mut::<Messages<aa_net::NetCommand>>()
        .drain()
        .map(|command| command.0)
        .collect();
    assert!(
        matches!(
            &sent[..],
            [aa_net::Command::UseMateSkill {
                skill_id: 20291,
                mate_obj_id: 20,
                ..
            }]
        ),
        "{sent:?}"
    );
    Ok(())
}

/// A driver slot is held back locally during the slave's global cooldown and until the press has an
/// answer, like the player's own bar; neither reaches the server.
#[test]
fn a_driver_slot_waits_for_the_slaves_gcd_and_its_answer() -> Result<(), Box<dyn std::error::Error>>
{
    use crate::hud::{
        alerts::UseFailed,
        usable::{Env, InFlight},
    };
    use crate::ingame::ground_target::SkillPressed;
    use bevy::ecs::system::SystemState;
    let (_rt, state, mut world) = driver_setup()?;
    // An accepted cast starts the slave's 5 s global cooldown.
    world
        .resource_mut::<aa_net::state::Skills>()
        .apply(&mount_started(20, 20291, 9, 0));
    let mut flight = InFlight::default();
    let press = |world: &mut World, flight: &mut InFlight| {
        let skills = world.resource::<aa_net::state::Skills>().clone();
        let mut writers: SystemState<(
            MessageWriter<SkillPressed>,
            MessageWriter<aa_net::NetCommand>,
            MessageWriter<UseFailed>,
        )> = SystemState::new(world);
        let (mut pressed, mut cast, mut fail) = writers.get_mut(world).unwrap();
        let data = state.borrow();
        let env = Env::of(&data, Some(&skills), &[], None);
        let mut target = aa_net::state::Target::default();
        target.obj_id = Some(777);
        use_slot(
            &data,
            1,
            &env,
            flight,
            &target,
            &mut pressed,
            &mut cast,
            &mut fail,
        );
    };
    let drain = |world: &mut World| {
        let sent = world
            .resource_mut::<Messages<aa_net::NetCommand>>()
            .drain()
            .count();
        let failed = world.resource_mut::<Messages<UseFailed>>().drain().count();
        (sent, failed)
    };
    // The slave's GCD refuses the press silently.
    press(&mut world, &mut flight);
    assert_eq!(drain(&mut world), (0, 0));
    // The GCD is over: the press goes out and is remembered.
    world
        .resource_mut::<aa_net::state::Skills>()
        .mount_last_cast
        .insert(
            20,
            (
                web_time::Instant::now() - std::time::Duration::from_secs(10),
                20291,
            ),
        );
    press(&mut world, &mut flight);
    assert_eq!(drain(&mut world), (1, 0));
    // The double press reaches neither the server nor a refusal alert, and the guard is pending.
    press(&mut world, &mut flight);
    assert_eq!(drain(&mut world), (0, 0));
    let skills = world.resource::<aa_net::state::Skills>().clone();
    assert!(flight.pending_mount(&skills, 20, web_time::Instant::now()));
    // The slave's cast starts: that answers the press.
    world
        .resource_mut::<aa_net::state::Skills>()
        .apply(&mount_started(20, 20291, 10, 0));
    let skills = world.resource::<aa_net::state::Skills>().clone();
    assert!(!flight.pending_mount(&skills, 20, web_time::Instant::now()));
    Ok(())
}

/// A player that spawns already on the driver seat, before the catalog is loaded: `ready` (called every
/// frame) must publish the bar once, not queue the driver payload again while `buff_skills` is still None.
#[test]
fn a_seated_driver_ready_publishes_the_bar_once() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(false)?;
    rt.lua
        .load("DRAWN=0;modeSkillActionBar={ShowModeSkillActionBar=function() DRAWN=DRAWN+1 end}")
        .exec()?;
    crate::hud::enchant::dispatch(
        None,
        &state,
        &Event::UnitSpawned(Box::new(slave_unit(20, 60))),
        None,
    );
    crate::hud::enchant::dispatch(
        None,
        &state,
        &Event::PlayerSpawn(Box::new(crate::hud::expansion::fixture::spawn())),
        None,
    );
    crate::hud::enchant::dispatch(
        None,
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    state.borrow().game.0.set(game()).ok().unwrap();
    for _ in 0..10 {
        super::ready(Some(&rt), &state);
    }
    assert_eq!(
        skill_ids(actions(&state.borrow())),
        [20704, 20291, 15622, 15450, 15546]
    );
    let drawn: usize = rt.lua.load("return DRAWN").eval()?;
    assert_eq!(drawn, 1);
    Ok(())
}

#[test]
fn native_cap_follows_existing_actions_and_keeps_duplicates() {
    let proof: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tools/evidence/user_music_actions/followup-cases.json"
    ))
    .unwrap();
    let controls = proof["refresh"]["cap_after_base"]
        .as_array()
        .or_else(|| proof["callbacks"]["cap_after_base"].as_array());
    // These exact original rows are also retained in followup-cases.json.
    let mut checked = 0;
    if let Some(controls) = controls {
        for row in controls {
            let mut actions =
                vec![Entry::Player(9); row["base_actions"].as_u64().unwrap() as usize];
            let entries: Vec<Entry> = [1, 1, 2, 3, 4, 5, 6, 7]
                .into_iter()
                .map(Entry::Player)
                .collect();
            append(&mut actions, &entries);
            assert_eq!(actions.len(), row["total"].as_u64().unwrap() as usize);
            checked += 1;
        }
    }
    assert_eq!(checked, 5, "native cap controls must be present");
    let mut actions = vec![];
    append(&mut actions, &[0, 1, 1, 0, 2].map(Entry::Player));
    assert_eq!(
        actions,
        [Entry::Player(1), Entry::Player(1), Entry::Player(2)]
    );
}

#[test]
fn initial_binding_publishes_old_then_new_count_and_updates_do_not_refresh()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(true)?;
    spawn(&state, Some(&rt), vec![buff(5068, 9, BuffKind::Good)]);
    rt.lua.load("assert(#OBS==2 and OBS[1][1]=='BUFF_SKILL_CHANGED' and OBS[1][2]==0 and OBS[2][1]=='MODE_ACTIONS_UPDATE' and OBS[2][2]==8)").exec()?;
    receipt(
        &rt,
        &state,
        Event::BuffUpdated {
            obj_id: 100,
            index: 9,
            stacks: 2,
            charged: 3,
        },
    );
    let mut repeated = crate::hud::expansion::fixture::spawn().unit;
    repeated.buffs = vec![];
    receipt(&rt, &state, Event::OwnUnitSnapshot(Box::new(repeated)));
    rt.lua
        .load("assert(#OBS==2 and X2Unit:GetModeActionsCount()==8)")
        .exec()?;
    // A real eligible application is never equality-suppressed, and promotes mode1.
    state.borrow_mut().input_modes.set(4, 1);
    receipt(&rt, &state, created(5068, 10));
    assert_eq!(
        state.borrow().input_modes.priority().next(),
        Some("mode_action_mode")
    );
    rt.lua
        .load("assert(#OBS==4 and OBS[3][2]==8 and OBS[4][2]==8)")
        .exec()?;
    Ok(())
}

#[test]
fn removal_excludes_all_colliding_instances_but_erases_only_first()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(true)?;
    spawn(
        &state,
        Some(&rt),
        vec![
            buff(3, 7, BuffKind::Hidden),
            buff(2, 7, BuffKind::Bad),
            buff(1, 7, BuffKind::Good),
            buff(1, 0, BuffKind::Good),
        ],
    );
    assert_eq!(players(actions(&state.borrow())), [71, 72, 73]);
    receipt(
        &rt,
        &state,
        Event::BuffRemoved {
            obj_id: 100,
            index: 7,
        },
    );
    // Removal excludes7, so native row0 participates in this publication.
    assert_eq!(players(actions(&state.borrow())), [71]);
    assert_eq!(
        crate::hud::user_music::own_buffs(&state.borrow())
            .unwrap()
            .len(),
        3
    );
    receipt(&rt, &state, created(5068, 9));
    assert_eq!(
        players(actions(&state.borrow())),
        [
            20848, 20849, 20850, 20851, 20852, 20853, 20854, 20855, 72, 73
        ]
    );
    Ok(())
}

#[test]
fn startup_replays_receipts_and_decode_loss_needs_snapshot_without_native_notifications()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(false)?;
    spawn(&state, Some(&rt), vec![]);
    receipt(&rt, &state, created(5068, 9));
    receipt(
        &rt,
        &state,
        Event::BuffRemoved {
            obj_id: 100,
            index: 9,
        },
    );
    assert!(actions(&state.borrow()).is_empty());
    state.borrow().game.0.set(game()).ok().unwrap();
    crate::hud::user_music::refresh_catalog(&state);
    ready(Some(&rt), &state);
    assert!(actions(&state.borrow()).is_empty());
    rt.lua
        .load("assert(#OBS==6 and OBS[4][2]==8 and OBS[6][2]==0)")
        .exec()?;
    receipt(&rt, &state, created(5068, 10));
    let gap = Event::UnhandledGamePacket {
        level: aa_net::game::LEVEL_COMPRESSED,
        opcode: 0xffff,
        len: 1,
    };
    receipt(&rt, &state, gap);
    receipt(&rt, &state, created(6010, 11));
    assert!(actions(&state.borrow()).is_empty());
    rt.lua.load("SAVED=#OBS").exec()?;
    let mut unit = crate::hud::expansion::fixture::spawn().unit;
    unit.buffs = vec![buff(6010, 11, BuffKind::Hidden)];
    receipt(&rt, &state, Event::OwnUnitSnapshot(Box::new(unit)));
    assert_eq!(players(actions(&state.borrow())), [22213, 22214, 22217]);
    rt.lua.load("assert(#OBS==SAVED)").exec()?;
    Ok(())
}

#[test]
fn callback_reentry_has_no_borrow_and_retired_context_cannot_resurrect()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(true)?;
    let shared = state.clone();
    rt.lua.globals().set(
        "PROMOTE",
        rt.lua.create_function(move |_, ()| {
            shared.borrow_mut().input_modes.set(4, 1);
            Ok(())
        })?,
    )?;
    rt.lua
        .load("REENTER=function(e) if e=='BUFF_SKILL_CHANGED' then PROMOTE() end end")
        .exec()?;
    spawn(&state, Some(&rt), vec![buff(5068, 9, BuffKind::Good)]);
    assert_eq!(
        state.borrow().input_modes.priority().collect::<Vec<_>>(),
        ["mode_action_mode", "quest_directing_mode"]
    );
    let shared = state.clone();
    rt.lua.globals().set(
        "RETIRE",
        rt.lua.create_function(move |_, ()| {
            crate::hud::user_music::receipt(None, &shared, &Event::UnitsRemoved(vec![100]));
            Ok(())
        })?,
    )?;
    rt.lua
        .load("REENTER=function(e) if e=='BUFF_SKILL_CHANGED' then RETIRE() end end")
        .exec()?;
    receipt(&rt, &state, created(6010, 10));
    assert!(actions(&state.borrow()).is_empty());
    Ok(())
}

#[test]
fn restored_hud_repaints_suspended_receipts_without_replay_or_promotion()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(true)?;
    rt.lua.load("modeSkillActionBar={ShowModeSkillActionBar=function() DRAWN=X2Unit:GetModeActionsCount() end}").exec()?;
    spawn(&state, Some(&rt), vec![buff(5068, 9, BuffKind::Good)]);
    crate::hud::user_music::receipt(
        None,
        &state,
        &Event::BuffRemoved {
            obj_id: 100,
            index: 9,
        },
    );
    assert!(actions(&state.borrow()).is_empty());
    state.borrow_mut().input_modes.set(4, 1);
    rt.lua.load("SAVED=#OBS").exec()?;
    ready(Some(&rt), &state);
    rt.lua.load("assert(DRAWN==0 and #OBS==SAVED)").exec()?;
    assert_eq!(
        state.borrow().input_modes.priority().collect::<Vec<_>>(),
        ["quest_directing_mode"]
    );
    // A suspended addition must not re-promote mode1 when Lua returns either.
    crate::hud::user_music::receipt(None, &state, &created(5068, 10));
    state.borrow_mut().input_modes.set(4, 1);
    ready(Some(&rt), &state);
    rt.lua.load("assert(DRAWN==8 and #OBS==SAVED)").exec()?;
    assert_eq!(
        state.borrow().input_modes.priority().collect::<Vec<_>>(),
        ["quest_directing_mode", "mode_action_mode"]
    );
    Ok(())
}

#[test]
fn slot_activation_keeps_current_target_and_unit_caster_intent()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::hud::{
        alerts::UseFailed,
        usable::{Env, InFlight},
    };
    use crate::ingame::ground_target::SkillPressed;
    use bevy::ecs::system::SystemState;
    let (rt, state) = runtime(true)?;
    spawn(&state, Some(&rt), vec![buff(6010, 9, BuffKind::Hidden)]);
    let mut world = World::new();
    world.init_resource::<Messages<SkillPressed>>();
    world.init_resource::<Messages<aa_net::NetCommand>>();
    world.init_resource::<Messages<UseFailed>>();
    let mut writers: SystemState<(
        MessageWriter<SkillPressed>,
        MessageWriter<aa_net::NetCommand>,
        MessageWriter<UseFailed>,
    )> = SystemState::new(&mut world);
    for target_obj in [None, Some(99), Some(777)] {
        let data = state.borrow();
        let env = Env::of(&data, None, &[], None);
        let mut target = aa_net::state::Target::default();
        target.obj_id = target_obj;
        let (mut pressed, mut cast, mut failed) = writers.get_mut(&mut world).unwrap();
        use_slot(
            &data,
            0,
            &env,
            &mut InFlight::default(),
            &target,
            &mut pressed,
            &mut cast,
            &mut failed,
        );
        use_slot(
            &data,
            15,
            &env,
            &mut InFlight::default(),
            &target,
            &mut pressed,
            &mut cast,
            &mut failed,
        );
    }
    let sent: Vec<_> = world
        .resource_mut::<Messages<SkillPressed>>()
        .drain()
        .collect();
    assert_eq!(sent.len(), 3);
    for (sent, target) in sent.iter().zip([0, 99, 777]) {
        assert_eq!(
            (sent.skill, sent.item, sent.target_obj_id),
            (22213, None, target)
        );
    }
    assert!(
        world
            .resource_mut::<Messages<UseFailed>>()
            .drain()
            .next()
            .is_none()
    );
    Ok(())
}

#[test]
#[ignore = "original mode-action bytecode, icons, tooltip and slot click integration"]
fn original_mode_bar_queries_granted_skills_hotkeys_and_clicks()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = crate::hud::user_music::fixture::runtime()?;
    for event in [created(5068, 9), created(6010, 10)] {
        crate::hud::enchant::dispatch(Some(&rt), &state, &event, None);
    }
    rt.lua
        .load(
            r#"
      assert(X2Unit:GetModeActionsCount()==11 and modeSkillActionBar:IsVisible())
      assert(HUD.mode_slot(1).id==20848 and HUD.mode_slot(9).id==22213)
      assert(HUD.mode_slot(0)==nil and HUD.mode_slot(16)==nil)
      local c=AAUI.SlotContent(ISLOT_MODE_ACTION,1)
      assert(c.type=='skill' and c.learned and c.icon~=nil and c.tooltip~=nil)
      assert(AAUI.SlotHotKey(ISLOT_MODE_ACTION,1)=='R')
      AAUI.UseSlot(ISLOT_MODE_ACTION,9,'LeftButton')
    "#,
        )
        .exec()?;
    assert!(
        state
            .borrow()
            .requests
            .iter()
            .any(|r| matches!(r, crate::hud::api::Request::UseModeAction { slot: 9 }))
    );
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    // Mode-only cooldown/usability receipts must requery already-bound slots,
    // independently of ordinary action bars and inventory panels.
    rt.lua.load(r#"
      MODE_QUERIES=0
      local previous=AAUI.SlotContent
      AAUI.SlotContent=function(kind,index)
        local c=previous(kind,index)
        if kind==ISLOT_MODE_ACTION then
          MODE_QUERIES=MODE_QUERIES+1
          if index==9 then MODE_CD=c and c.cooldown and c.cooldown.remain;MODE_CAN_USE=c and c.canUse end
        end
        return c
      end
    "#).exec()?;
    let panels = state.borrow().panels.clone();
    let mut world = World::new();
    let mut skills = aa_net::state::Skills {
        obj_id: 100,
        ..default()
    };
    skills
        .native_skill_cooldowns
        .push(aa_net::state::NativeCooldown {
            cooldown: aa_net::Cooldown {
                id: 22213,
                total_ms: 5000,
                remaining_ms: 5000,
            },
            updated_at: web_time::Instant::now(),
            origin: aa_net::state::NativeCooldownOrigin::Snapshot,
            reset: false,
        });
    world.insert_resource(skills);
    refresh_slots(&world, &mut state.borrow_mut());
    assert!(state.borrow().mode_actions.slots_dirty);
    push_slots(&rt, &state);
    rt.lua.load("assert(MODE_QUERIES>0 and MODE_CD>0)").exec()?;
    world
        .resource_mut::<aa_net::state::Skills>()
        .native_skill_cooldowns
        .clear();
    {
        let mut data = state.borrow_mut();
        data.snapshot.dead = true;
        data.snapshot.hp = 0;
    }
    refresh_slots(&world, &mut state.borrow_mut());
    assert!(state.borrow().mode_actions.slots_dirty);
    // Keep the refresh pending through suspension, then restore the UI without
    // a mode-list notification or input-mode promotion.
    state.borrow_mut().input_modes.set(4, 1);
    push_slots(&rt, &state);
    rt.lua
        .load("assert(MODE_CD==nil and MODE_CAN_USE==false)")
        .exec()?;
    assert_eq!(state.borrow().panels, panels);
    assert_eq!(
        state.borrow().input_modes.priority().next(),
        Some("quest_directing_mode")
    );
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// The real gunner bars: the point-1 attached skill is shown (target_type 0), the linked mount
/// skill is cast. For a cannon that cast is BallisticPos (11), so a published sight makes it an
/// aimed `UseMateSkillAt`; the tower deployment skill (slave 100) is not a shot and keeps the
/// unit target.
#[test]
#[ignore = "requires AA_DB and AA_PAK"]
fn the_real_gunner_bars_cast_the_aimed_fire_skill() {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    if std::env::var("AA_PAK").is_err() || !std::path::Path::new(&path).exists() {
        return;
    }
    let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("read the game database");
    let tables = Tables::load(&conn).expect("mode action tables");
    let game = crate::hud::data::GameData::load(&path).expect("skill catalog");
    let target_type = |skill: u32| game.skills.get(&skill).map(|s| s.target_type);
    for (template, display, mount, fire, aimed) in [
        (10u32, 14372u32, 16u32, 10473u32, true),
        (133, 24501, 341, 24500, true),
        (100, 20161, 184, 20160, false),
    ] {
        let bar = tables.gunner_bar(template).expect("gunner bar");
        let entry = bar.first().expect("one gunner slot");
        assert_eq!(
            (entry.skill, entry.mount_skill, entry.fire),
            (display, mount, Some(fire))
        );
        assert_eq!(
            target_type(display),
            Some(0),
            "the bar skill is not the shot"
        );
        let sight = crate::ingame::turret::Sight {
            slave: 20,
            aim: aa_net::TurretAim {
                pitch: 0.3,
                yaw: -0.2,
            },
            pos: aa_net::Pos::new(130.25, 131.5, 100.0),
            rot: 0.7,
        };
        let command = slave_cast(entry, Some(&game), Some(sight), 20, 0);
        if aimed {
            assert_eq!(
                target_type(fire),
                Some(11),
                "the cast skill is BallisticPos"
            );
            let aa_net::Command::UseMateSkillAt {
                skill_id,
                mate_obj_id,
                mount_skill_id,
                pos,
                rot,
            } = command
            else {
                panic!("template {template}: {command:?}")
            };
            assert_eq!(
                (skill_id, mate_obj_id, mount_skill_id, pos, rot),
                (fire, 20, mount, sight.pos, sight.rot)
            );
        } else {
            let aa_net::Command::UseMateSkill {
                skill_id,
                mate_obj_id,
                mount_skill_id,
                target_obj_id,
            } = command
            else {
                panic!("template {template}: {command:?}")
            };
            assert_eq!(
                (skill_id, mate_obj_id, mount_skill_id, target_obj_id),
                (fire, 20, mount, 0)
            );
        }
    }
}

/// A row with an attached list counts only at the FIRST entry's point, on every seat: mount
/// skill 188 has `mount_attached_skills` 35 then 1 (row ids 299, 300), so it counts at 35 and
/// never at point 1, which a point-only lookup would accept.
#[test]
fn the_first_attached_entry_chooses_the_seat() {
    let mut tables = Tables::default();
    tables
        .slave_skills
        .insert(5, vec![SlaveSkill::new(20161, 188)]);
    tables.attached.insert((5, 188, 35), 20161);
    tables.attached.insert((5, 188, 1), 20161);
    tables.first_attached.insert((5, 188), 35);
    assert!(
        tables.bar(5, 1, false).expect("rows").is_empty(),
        "point 1 is not the first attached entry"
    );
    assert_eq!(
        tables
            .bar(5, 35, false)
            .expect("rows")
            .iter()
            .map(|row| row.mount_skill)
            .collect::<Vec<_>>(),
        [188]
    );
}

/// Every kind-7/8 driver bar applies the native seat filter (`FUN_39347260`): a row without an
/// attached list counts at point 1, a row whose FIRST attached entry is another point does not
/// (tank 53's row 39 at point 2, slave 54's row 44, ladder 5's row 236 at point 81). The kept
/// rows are unchanged otherwise: a turret shows its point-1 attached skill and casts the mount
/// skill (the cotton cart 73's mount 65 has attached 15548 at point 1 but is not a turret, so it
/// keeps 15622), and the harpoon rows without an attached list stay.
#[test]
#[ignore = "requires AA_DB"]
fn every_real_kind_7_8_bar_filters_by_the_seat_and_only_turrets_show_attached_skills() {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    if !std::path::Path::new(&path).exists() {
        return;
    }
    let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("read the game database");
    let tables = Tables::load(&conn).expect("mode action tables");
    let Some(db) = crate::ingame::data::Db::open() else {
        return;
    };
    let mut query = conn
        .prepare(
            "SELECT id, name, slave_kind_id, model_id FROM slaves WHERE slave_kind_id IN (7, 8)",
        )
        .expect("slave query");
    let rows = query
        .query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, u32>(3)?,
            ))
        })
        .expect("query slaves")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("read slaves");
    let mut checked = 0;
    let mut dropped = 0;
    for (id, name, kind, model) in rows {
        let mounts = tables.slave_skills.get(&id).cloned().unwrap_or_default();
        let turret = crate::ingame::turret::turret_seat_model(kind, model, &db);
        let bar = tables.bar(id, 1, turret).unwrap_or_default();
        let level = tables.slave_levels.get(&id).copied().unwrap_or(u8::MAX);
        let expected = mounts
            .iter()
            .filter(|row| tables.skill_levels.get(&row.skill).copied().unwrap_or(1) <= level)
            .filter(|row| {
                tables
                    .first_attached
                    .get(&(id, row.mount_skill))
                    .is_none_or(|first| *first == 1)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            bar.len(),
            expected.len(),
            "slave {id} {name} keeps the rows of its seat"
        );
        for (entry, mount) in bar.iter().zip(&expected) {
            assert_eq!(entry.mount_skill, mount.mount_skill);
            match tables.attached.get(&(id, mount.mount_skill, 1)) {
                Some(attached) if turret => {
                    assert_eq!(entry.skill, *attached, "slave {id} {name} attached skill");
                    assert_eq!(
                        entry.fire,
                        Some(mount.skill),
                        "slave {id} {name} fire skill"
                    );
                }
                _ => {
                    assert_eq!(entry.skill, mount.skill, "slave {id} {name} mount skill");
                    assert_eq!(entry.fire, None, "slave {id} {name} has no attached fire");
                }
            }
        }
        dropped += mounts.len() - expected.len();
        checked += 1;
    }
    assert!(checked >= 40, "checked {checked} kind-7/8 rows");
    assert!(
        dropped >= 3,
        "the point-2 and point-81 rows are filtered out ({dropped} dropped)"
    );

    let cart = tables.bar(73, 1, false).expect("cotton cart bar");
    assert_eq!(cart[0].skill, 15622, "the cart keeps its mount skill");
    let tank = tables.bar(53, 1, false).expect("tank 53 bar");
    assert!(
        tank.iter().all(|row| row.mount_skill != 39),
        "tank 53 drops the point-2 row 39"
    );
    let ladder = tables.bar(5, 1, false).expect("siege ladder bar");
    assert!(
        ladder.iter().all(|row| row.mount_skill != 236),
        "ladder 5 drops the point-81 row 236"
    );
    let tank54 = tables.bar(54, 1, true).expect("tank 54 bar");
    assert!(
        tank54.iter().all(|row| row.mount_skill != 44),
        "tank 54 drops the point-2 row on the driver seat"
    );
    let tank54_passenger = tables.bar(54, 2, false).expect("tank 54 passenger bar");
    assert_eq!(
        tank54_passenger
            .iter()
            .map(|row| (row.skill, row.mount_skill, row.cast_skill()))
            .collect::<Vec<_>>(),
        [(14299, 44, 14344)],
        "the point-2 seat shows the attached 14299 and casts 14344"
    );
    for id in [37u32, 48] {
        assert!(
            tables
                .bar(id, 1, true)
                .expect("harpoon bar")
                .iter()
                .any(|row| row.skill == 13750),
            "harpoon {id} keeps 13750"
        );
    }
    assert_eq!(
        tables.bar(90, 1, true).expect("harpoon bar").len(),
        2,
        "slave 90 keeps both rows"
    );
    for (id, display, fire, mount) in [
        (10u32, 14372u32, 10473u32, 16u32),
        (133, 24501, 24500, 341),
        (100, 20161, 20160, 184),
    ] {
        let bar = tables.bar(id, 1, true).expect("gunner bar");
        let entry = bar
            .iter()
            .find(|row| row.mount_skill == mount)
            .unwrap_or_else(|| panic!("slave {id} mount {mount}"));
        assert_eq!(
            (entry.skill, entry.fire),
            (display, Some(fire)),
            "slave {id}"
        );
    }
}

fn mate_unit(id: u32) -> aa_net::model::Unit {
    let mut unit = crate::hud::expansion::fixture::spawn().unit;
    unit.obj_id = id;
    unit.kind = aa_net::model::UnitKind::Mate {
        tl_id: 8,
        template_id: 1,
        owner_id: 100,
    };
    unit
}

/// `FUN_392045f0` puts the mate command 9 function slot before the buff skills when the ridden
/// mate is not the local player's own (`FUN_3933b1b0(parent) == 0`); the slave could not supply
/// one (no seat), so this is the only function slot on that mate's bar.
#[test]
fn a_foreign_mate_rider_gets_command_unmount_before_the_buff_skills()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(true)?;
    let mut own = crate::hud::expansion::fixture::spawn();
    own.unit.buffs = vec![buff(5068, 9, BuffKind::Good)];
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitSpawned(Box::new(mate_unit(40))),
        None,
    );
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 40,
            point: 1,
            reason: 0,
        },
        None,
    );
    assert!(state.borrow().mounted.riding_mate());
    assert!(
        state.borrow().mounted.mate_unmount(),
        "a foreign mate's slot"
    );
    let bar = state.borrow();
    assert_eq!(
        functions(actions(&bar)),
        [Function::CommandUnmount],
        "mate command 9 is the first slot"
    );
    assert_eq!(
        players(actions(&bar)),
        (20848..=20855).collect::<Vec<_>>(),
        "the buff skills follow it"
    );
    assert_eq!(actions(&bar).len(), 9);
    drop(bar);
    assert_eq!(
        rt.lua
            .load("return X2Unit:GetModeActionsCount()")
            .eval::<usize>()?,
        9
    );
    Ok(())
}

/// `FUN_3933b1b0` returns 1 for the player's own mate (kind 5 and the owner is the local
/// character), and `FUN_392045f0` adds `command_unmount` only when it returns 0: riding the own
/// pet shows the buff skills with no function slot (the pet bar owns the dismount).
#[test]
fn the_players_own_mate_gets_no_command_unmount_slot() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(true)?;
    let mut own = crate::hud::expansion::fixture::spawn();
    own.unit.buffs = vec![buff(5068, 9, BuffKind::Good)];
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    let mut mate = mate_unit(40);
    let aa_net::model::UnitKind::Mate { owner_id, .. } = &mut mate.kind else {
        unreachable!()
    };
    *owner_id = 47; // the fixture character
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::UnitSpawned(Box::new(mate)), None);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 40,
            point: 1,
            reason: 0,
        },
        None,
    );
    assert!(!state.borrow().mounted.mate_unmount());
    assert!(state.borrow().mounted.riding_mate());
    assert_eq!(functions(actions(&state.borrow())), []);
    assert_eq!(
        players(actions(&state.borrow())),
        (20848..=20855).collect::<Vec<_>>(),
        "the buff skills, as on foot"
    );
    assert_eq!(actions(&state.borrow()).len(), 8);
    Ok(())
}

/// `HUD.mode_slot` must not let the previous bar's cached slot stand in for a slot the current
/// bar computes as a different kind or id (one frame between the swap and the slot refresh).
#[test]
fn a_stale_mode_slot_cache_from_the_previous_bar_is_not_read()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(true)?;
    {
        let mut data = state.borrow_mut();
        data.mode_actions.actions = vec![Entry::Function(Function::Unbind)];
        data.mode_actions.slots = vec![crate::hud::panels::SlotSnap {
            kind: "skill",
            id: 20848,
            ..Default::default()
        }];
    }
    rt.lua
        .load(
            r#"
        local s=HUD.mode_slot(1)
        assert(s and s.kind=='function' and s.id==1, 'the function slot, not the old skill')
    "#,
        )
        .exec()?;
    // the cache is still used when it is that same slot (its cooldown end lives there)
    {
        let mut data = state.borrow_mut();
        data.mode_actions.actions = vec![Entry::Player(20848)];
        data.mode_actions.slots = vec![crate::hud::panels::SlotSnap {
            kind: "skill",
            id: 20848,
            name: "cached".into(),
            ..Default::default()
        }];
    }
    rt.lua
        .load("assert(HUD.mode_slot(1).name=='cached', 'the same slot keeps the cached snapshot')")
        .exec()?;
    Ok(())
}

#[test]
fn show_slave_info_queues_the_original_ui_event() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime(false)?;
    rt.lua
        .load(
            r#"
        INFO=0
        local w=UIParent:CreateWidget('emptywidget','slaveInfoProbe','UIParent')
        w:RegisterEvent('SHOW_SLAVE_INFO')
        w:SetHandler('OnEvent',function() INFO=INFO+1 end)
    "#,
        )
        .exec()?;
    let outcome = super::run_function(
        &mut state.borrow_mut(),
        Function::ShowSlaveInfo,
        web_time::Instant::now(),
    );
    assert_eq!(outcome, crate::hud::slave::Outcome::Sent);
    super::ready(Some(&rt), &state);
    assert_eq!(rt.lua.load("return INFO").eval::<i64>()?, 1);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    Ok(())
}

#[test]
fn function_slots_carry_the_native_icons_and_ui_texts_keys() {
    let mut tables = super::Tables::default();
    tables.texts.insert(
        (33, "unbind".into()),
        "Disembark from current device.".into(),
    );
    tables.texts.insert(
        (33, "showSlaveInfo".into()),
        "Check details on a summon.".into(),
    );
    tables
        .texts
        .insert((19, "command_unmount".into()), "Dismount".into());
    let unbind = function_slot(Some(&tables), Function::Unbind);
    assert_eq!(unbind.kind, "function");
    assert_eq!(unbind.icon, "ui/icon/icon_skill_vehicle03.dds");
    assert_eq!(unbind.desc, "Disembark from current device.");
    let info = function_slot(Some(&tables), Function::ShowSlaveInfo);
    assert_eq!(info.icon, "ui/icon/icon_skill_info_view.dds");
    assert_eq!(info.desc, "Check details on a summon.");
    let mate = function_slot(Some(&tables), Function::CommandUnmount);
    assert_eq!(mate.icon, "ui/icon/icon_skill_vehicle03.dds");
    assert_eq!(mate.desc, "Dismount");
    // without the text table the key is still the original key
    assert_eq!(function_slot(None, Function::Unbind).desc, "unbind");
}

/// The stock Summon Info window (`slave_info.alb`) opens from the mode bar's `showSlaveInfo`
/// slot: `SHOW_SLAVE_INFO` is the event `mode_action.alb` handles, and the window reads the
/// already-installed `X2SiegeWeapon:GetMountedSiegeWeaponInfo`. Run with the original pak and
/// database present; it lists what the client cannot fill in its own header (the window itself
/// hides missing vehicle/speed rows).
#[test]
#[ignore = "original World UI, game pak and database: Summon Info window"]
fn original_summon_info_window_opens_on_show_slave_info() -> Result<(), Box<dyn std::error::Error>>
{
    let (rt, state, path) = crate::hud::breath::fixture::runtime(1280., 720.)?;
    let own = crate::hud::expansion::fixture::spawn();
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::CharacterSelected(Box::new(own.state.clone())),
        None,
    );
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    let mut cart = crate::hud::expansion::fixture::spawn().unit;
    cart.obj_id = 20;
    cart.name = "Farm Wagon".into();
    cart.kind = aa_net::model::UnitKind::Slave {
        id: 700,
        tl_id: 8,
        template_id: 60,
        summoner_id: 100,
    };
    cart.model_id = 1008;
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::UnitSpawned(Box::new(cart)), None);
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    assert_eq!(
        rt.lua
            .load("return X2Unit:GetModeActionsCount()")
            .eval::<usize>()?,
        7
    );
    assert!(super::function_at(&state.borrow(), 6) == Some(Function::ShowSlaveInfo));
    super::run_function(
        &mut state.borrow_mut(),
        Function::ShowSlaveInfo,
        web_time::Instant::now(),
    );
    super::ready(Some(&rt), &state);
    rt.lua
        .load(
            r#"
        assert(slaveInfoFrame, 'stock Summon Info frame')
        assert(slaveInfoFrame:IsVisible(), 'SHOW_SLAVE_INFO opens it')
        assert(slaveInfoFrame.unitId == '14', 'the received slave key in lowercase hex')
        assert(slaveInfoFrame.name:GetText() == 'Farm Wagon')
        assert(slaveInfoFrame.class:GetText() ~= '', 'the descriptor class')
        assert(slaveInfoFrame.kind:GetText() ~= '', 'the mapped slave kind')
    "#,
        )
        .exec()?;
    aa_ui::lua::tick(&rt.lua, 16.);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    drop(rt);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
