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
    let db = Connection::open_with_flags(
        aa_anim::db::DEFAULT_DB,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
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
    world.init_resource::<Messages<UseFailed>>();
    let mut writers: SystemState<(MessageWriter<SkillPressed>, MessageWriter<UseFailed>)> =
        SystemState::new(&mut world);
    for target_obj in [None, Some(99), Some(777)] {
        let data = state.borrow();
        let env = Env::of(&data, None, &[], None);
        let mut target = aa_net::state::Target::default();
        target.obj_id = target_obj;
        let (mut pressed, mut failed) = writers.get_mut(&mut world).unwrap();
        use_slot(
            &data,
            0,
            &env,
            &InFlight::default(),
            &target,
            &mut pressed,
            &mut failed,
        );
        use_slot(
            &data,
            15,
            &env,
            &InFlight::default(),
            &target,
            &mut pressed,
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
            updated_at: std::time::Instant::now(),
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
