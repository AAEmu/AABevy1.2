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
        aa_anim::db::DEFAULT_DB,
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
        aa_anim::db::DEFAULT_DB,
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
    Ok(())
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
    // a passenger seat keeps the normal bar
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
    assert_eq!(actions(&state.borrow()).len(), 8);
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
    assert_eq!(
        actions(&state.borrow()),
        [20704, 20291, 15622, 15450, 15546]
    );
    assert_eq!(
        rt.lua
            .load("return X2Unit:GetModeActionsCount()")
            .eval::<usize>()?,
        5
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
    assert_eq!(actions(&state.borrow()), [15622, 17000]);
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
        actions(&state.borrow()),
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
        actions(&state.borrow()),
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
        actions(&state.borrow()),
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
        actions(&state.borrow()),
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
            let mut actions = vec![9; row["base_actions"].as_u64().unwrap() as usize];
            append(&mut actions, &[1, 1, 2, 3, 4, 5, 6, 7]);
            assert_eq!(actions.len(), row["total"].as_u64().unwrap() as usize);
            checked += 1;
        }
    }
    assert_eq!(checked, 5, "native cap controls must be present");
    let mut actions = vec![];
    append(&mut actions, &[0, 1, 1, 0, 2]);
    assert_eq!(actions, [1, 1, 2]);
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
    assert_eq!(actions(&state.borrow()), [71, 72, 73]);
    receipt(
        &rt,
        &state,
        Event::BuffRemoved {
            obj_id: 100,
            index: 7,
        },
    );
    // Removal excludes7, so native row0 participates in this publication.
    assert_eq!(actions(&state.borrow()), [71]);
    assert_eq!(
        crate::hud::user_music::own_buffs(&state.borrow())
            .unwrap()
            .len(),
        3
    );
    receipt(&rt, &state, created(5068, 9));
    assert_eq!(
        actions(&state.borrow()),
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
    assert_eq!(actions(&state.borrow()), [22213, 22214, 22217]);
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
