use super::*;
use crate::hud::alerts::UseFailed;
use crate::hud::api::Request;
use crate::hud::usable;
use crate::hud::usable::InFlight;

#[test]
fn pet_experience_keeps_threshold_equality_level100_and_float_rounding() {
    let tables = Tables {
        levels: vec![(1, 0), (2, 50), (3, 250), (100, 16_417_500), (101, 16_917_500)],
        ..default()
    };
    assert_eq!(tables.progress(-1).unwrap(), [0.0, -1.0, 0.0, 50.0]);
    assert_eq!(tables.progress(50).unwrap(), [0.0, 50.0, 50.0, 200.0]);
    assert_eq!(tables.progress(100).unwrap(), [25.0, 100.0, 50.0, 200.0]);
    assert_eq!(tables.progress(16_417_500).unwrap(), [0.0, 16_417_500.0, 16_417_500.0, 500_000.0]);
    assert_eq!(tables.progress(16_917_500).unwrap()[0], 100.0);
    assert_eq!(
        tables.progress(16_917_501).unwrap()[1],
        16_917_500.0,
        "each returned integer converts through f32"
    );
}

#[test]
fn pet_identity_rejects_metadata_without_unit_owner_and_timeline() {
    let (player, mut units, mut mates) = super::super::active_fixture::received();
    let owner = player.spawn.as_ref().unwrap().character().id;
    assert_eq!(current(&mates, &units, owner).unwrap().0.obj_id, 201);
    assert!(current(&mates, &units, owner + 1).is_none());
    units.by_id.get_mut(&201).unwrap().kind = UnitKind::Mate {
        tl_id: 13,
        template_id: 504,
        owner_id: owner,
    };
    assert!(current(&mates, &units, owner).is_none());
    units.by_id.remove(&201);
    assert!(current(&mates, &units, owner).is_none());
    mates.own.as_mut().unwrap().obj_id = None;
    assert!(current(&mates, &units, owner).is_none());
}

#[test]
fn displayed_skills_preserve_received_order_and_first_seat_only() {
    let mut tables = Tables::default();
    let mut add = |id, seat| {
        tables.skills.insert(
            id,
            Skill {
                combat: id + 100,
                first_seat: seat,
                level: 1,
                target_type: 0,
            },
        );
    };
    add(10, None);
    add(20, Some(1));
    add(30, Some(2));
    add(40, Some(0));
    let pet = Pet {
        spawn: MateSpawn {
            skills: [0, 30, 20, 99, 10, 40, 20, 0, 0, 0],
            ..default()
        },
        ..default()
    };
    assert_eq!((1..=4).map(|i| displayed(&tables, &pet, i).unwrap().0).collect::<Vec<_>>(), [2, 4, 5, 6]);
    assert!(displayed(&tables, &pet, 0).is_none() && displayed(&tables, &pet, 5).is_none());
    let mounted = Pet { driving: true, ..pet };
    assert_eq!((1..=3).map(|i| displayed(&tables, &mounted, i).unwrap().0).collect::<Vec<_>>(), [2, 4, 6]);
    assert!(displayed(&tables, &mounted, 4).is_none());
}

fn send_commands(
    mut net: MessageWriter<NetCommand>,
    bridge: NonSend<HudBridge>,
    player: Res<Player>,
    units: Res<Units>,
    mates: Res<aa_net::Mates>,
) {
    let mut data = bridge.0.borrow_mut();
    commands(&mut data, &player, &units, &mates, &mut net, &mut crate::ingame::MountReason::default());
}

fn casting_pet(level: u8, driving: bool) -> HudData {
    let mut game = super::super::super::data::GameData::default();
    game.pets.owned.skills.insert(
        7,
        Skill {
            combat: 700,
            first_seat: Some(1),
            level: 10,
            target_type: 4,
        },
    );
    game.pets.owned.skills.insert(
        8,
        Skill {
            combat: 800,
            first_seat: None,
            level: 1,
            target_type: 6,
        },
    );
    game.pets.owned.skills.insert(
        9,
        Skill {
            combat: 900,
            first_seat: Some(0),
            level: 1,
            target_type: 0,
        },
    );
    let mut data = HudData::default();
    data.game.0.set(game).ok();
    data.character_id = 47;
    data.pets.owned.current = Some(Pet {
        spawn: MateSpawn {
            tl_id: 12,
            skills: [7, 8, 9, 0, 0, 0, 0, 0, 0, 0],
            ..default()
        },
        object: 201,
        level,
        driving,
        attached_to_own: driving,
        ..default()
    });
    data
}

#[test]
fn riding_mount_skill_uses_the_mount_row_and_mate_caster() {
    let mut data = casting_pet(30, true);
    data.pets.owned.current.as_mut().unwrap().spawn.id = 71;
    assert_eq!(use_skill(&mut data, 1, 555), Ok(()));
    let queued = &data.pets.owned.casts;
    let (owner, identity, cast) = queued.first().expect("queued mount cast");
    assert_eq!((*owner, *identity), (47, (201, 12, 71)));
    assert_eq!(cast.skill_id, 700);
    assert_eq!(cast.mount_skill_id, 7);
    assert_eq!(cast.target_obj_id, 555);
    let (player, units, mates) = super::super::active_fixture::received();
    let bridge = HudBridge(std::rc::Rc::new(std::cell::RefCell::new(data)));
    let mut app = App::new();
    app.insert_resource(player)
        .insert_resource(units)
        .insert_resource(mates)
        .insert_non_send(bridge)
        .add_message::<NetCommand>()
        .add_systems(Update, send_commands);
    app.update();
    let mut messages = app.world_mut().resource_mut::<Messages<NetCommand>>();
    let sent: Vec<Command> = messages.drain().map(|c| c.0).collect();
    assert_eq!(sent.len(), 1, "nothing is sent for an attached rider skill");
    let [command] = &sent[..] else {
        panic!("{sent:?}")
    };
    let Command::UseMateSkill {
        skill_id,
        mate_obj_id,
        mount_skill_id,
        target_obj_id,
    } = command
    else {
        panic!("{command:?}")
    };
    assert_eq!(skill_id, &700);
    assert_eq!(mate_obj_id, &201);
    assert_eq!(mount_skill_id, &7);
    assert_eq!(target_obj_id, &555);
    let body = command
        .mate_skill_message(100)
        .expect("the mount cast message")
        .encode_body();
    assert_eq!(body.len(), 17);
    assert_eq!(
        body,
        [
            0xbc, 0x02, 0, 0, 3, 0xc9, 0, 0, 7, 0, 0, 0, 0, 0x2b, 0x02, 0, 0
        ]
    );
}

#[test]
fn pet_action_hotkey_casts_through_handle_requests_and_encodes_the_command() {
    let mut data = casting_pet(30, true);
    data.pets.owned.current.as_mut().unwrap().spawn.id = 71;
    data.requests.push(Request::UsePetAction { slot: 1 });
    let (player, units, mates) = super::super::active_fixture::received();
    let mut target = aa_net::state::Target::default();
    target.obj_id = Some(555);
    let bridge = HudBridge(std::rc::Rc::new(std::cell::RefCell::new(data)));
    let mut app = App::new();
    app.insert_non_send(bridge)
        .init_resource::<NextState<crate::Screen>>()
        .init_resource::<aa_net::state::ActionBar>()
        .init_resource::<aa_net::state::Inventory>()
        .init_resource::<aa_net::state::Skills>()
        .init_resource::<aa_net::state::Buffs>()
        .init_resource::<InFlight>()
        .insert_resource(target)
        .insert_resource(player)
        .insert_resource(units)
        .insert_resource(mates)
        .add_message::<NetCommand>()
        .add_message::<UseFailed>()
        .add_message::<crate::ingame::ground_target::SkillPressed>()
        .add_systems(
            Update,
            (super::super::super::api::handle_requests, send_commands).chain(),
        );
    app.update();
    let sent: Vec<Command> = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|c| c.0)
        .collect();
    let [command] = &sent[..] else {
        panic!("{sent:?}")
    };
    let Command::UseMateSkill {
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
        (700, 201, 7, 555)
    );
    assert!(
        app.world().resource::<Messages<UseFailed>>().is_empty(),
        "the happy path shows no refusal"
    );
    assert_eq!(
        command
            .mate_skill_message(100)
            .expect("the mount cast message")
            .encode_body(),
        [
            0xbc, 0x02, 0, 0, 3, 0xc9, 0, 0, 7, 0, 0, 0, 0, 0x2b, 0x02, 0, 0
        ]
    );
}

#[test]
fn self_targeted_mount_skill_targets_the_mate() {
    let mut data = casting_pet(30, true);
    assert_eq!(use_skill(&mut data, 2, 555), Ok(()));
    assert_eq!(data.pets.owned.casts[0].2.target_obj_id, 201);
}

#[test]
fn low_level_and_attach_mismatch_refuse_with_not_now() {
    let mut data = casting_pet(5, true);
    assert_eq!(use_skill(&mut data, 1, 555), Err(usable::code::NOT_NOW));
    assert!(data.pets.owned.casts.is_empty());
    let mut data = casting_pet(30, false);
    assert_eq!(use_skill(&mut data, 1, 555), Err(usable::code::NOT_NOW));
    assert!(
        data.pets.owned.casts.is_empty(),
        "the driver-only row is not attached for the riding flag 0"
    );
}

#[test]
fn casting_on_foot_refuses_with_not_now_and_sends_nothing() {
    let mut data = casting_pet(30, false);
    assert_eq!(use_skill(&mut data, 1, 555), Err(usable::code::NOT_NOW));
    assert_eq!(
        use_skill(&mut data, 2, 555),
        Err(usable::code::NOT_NOW),
        "live capture 2026-10-28: every slot is the local 0x2c refusal on foot, even a free row"
    );
    assert!(data.pets.owned.casts.is_empty());
}

#[test]
fn mount_skill_display_is_usable_only_while_riding() {
    let pet = |driving| Pet {
        driving,
        ..default()
    };
    let skill = |target_type| Skill {
        combat: 1,
        first_seat: Some(1),
        level: 1,
        target_type,
    };
    assert!(!usable(&skill(0), &pet(false)));
    assert!(usable(&skill(0), &pet(true)));
    assert!(
        usable(&skill(6), &pet(false)),
        "target_type_id 6 skips the general validator (3933b7a7)"
    );
}

fn captured_pet() -> HudData {
    let mut game = super::super::super::data::GameData::default();
    for (mount, combat, level) in [(83, 17092, 5), (1, 11016, 10), (13, 11369, 15)] {
        game.pets.owned.skills.insert(
            mount,
            Skill {
                combat,
                first_seat: Some(1),
                level,
                target_type: 0,
            },
        );
    }
    let mut data = HudData::default();
    data.game.0.set(game).ok();
    data.character_id = 47;
    data.pets.owned.current = Some(Pet {
        spawn: MateSpawn {
            tl_id: 12,
            id: 71,
            skills: [83, 1, 13, 0, 0, 0, 0, 0, 0, 0],
            ..default()
        },
        object: 0x5581,
        level: 40,
        driving: true,
        attached_to_own: true,
        ..default()
    });
    data
}

fn captured_fixture() -> (Player, Units, Mates) {
    let (player, mut units, mut mates) = super::super::active_fixture::received();
    let mut unit = units.by_id.remove(&201).unwrap();
    unit.obj_id = 0x5581;
    units.by_id.insert(0x5581, unit);
    mates.own.as_mut().unwrap().obj_id = Some(0x5581);
    (player, units, mates)
}

fn send_captured_cast(mut data: HudData, slot: usize, target_obj_id: u32) -> Vec<u8> {
    assert_eq!(use_skill(&mut data, slot, target_obj_id), Ok(()));
    let (player, units, mates) = captured_fixture();
    let bridge = HudBridge(std::rc::Rc::new(std::cell::RefCell::new(data)));
    let mut app = App::new();
    app.insert_resource(player)
        .insert_resource(units)
        .insert_resource(mates)
        .insert_non_send(bridge)
        .add_message::<NetCommand>()
        .add_systems(Update, send_commands);
    app.update();
    let mut messages = app.world_mut().resource_mut::<Messages<NetCommand>>();
    let sent: Vec<Command> = messages.drain().map(|c| c.0).collect();
    let [command] = &sent[..] else {
        panic!("{sent:?}")
    };
    command
        .mate_skill_message(100)
        .expect("the mount cast message")
        .encode_body()
}

#[test]
fn captured_self_cast_matches_the_wire() {
    assert_eq!(
        send_captured_cast(captured_pet(), 1, 0),
        [
            0xc4, 0x42, 0, 0, 3, 0x81, 0x55, 0, 0x53, 0, 0, 0, 0, 0, 0, 0, 0
        ],
        "live capture 2026-10-28: skill 17092, mate 0x5581, mount_skill 83"
    );
}

#[test]
fn captured_targeted_cast_matches_the_wire() {
    assert_eq!(
        send_captured_cast(captured_pet(), 2, 0x6703),
        [
            0x08, 0x2b, 0, 0, 3, 0x81, 0x55, 0, 1, 0, 0, 0, 0, 0x03, 0x67, 0, 0
        ],
        "live capture 2026-10-28: skill 11016, mate 0x5581, mount_skill 1, target 0x6703"
    );
}

#[test]
fn captured_no_target_cast_still_sends_object_zero() {
    assert_eq!(
        send_captured_cast(captured_pet(), 2, 0),
        [
            0x08, 0x2b, 0, 0, 3, 0x81, 0x55, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0
        ],
        "live capture 2026-10-28: a targeted row with no current target still casts"
    );
}

fn pet_action_app(data: HudData) -> App {
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(data));
    let mut app = App::new();
    app.insert_non_send(HudBridge(bridge))
        .init_resource::<NextState<crate::Screen>>()
        .init_resource::<aa_net::state::Units>()
        .init_resource::<aa_net::state::ActionBar>()
        .init_resource::<aa_net::state::Inventory>()
        .init_resource::<aa_net::state::Target>()
        .init_resource::<aa_net::state::Skills>()
        .init_resource::<aa_net::state::Buffs>()
        .init_resource::<InFlight>()
        .add_message::<NetCommand>()
        .add_message::<UseFailed>()
        .add_message::<crate::ingame::ground_target::SkillPressed>()
        .add_systems(Update, super::super::super::api::handle_requests);
    app
}

#[test]
fn pet_action_without_a_mate_or_game_tables_is_silent() {
    let mut data = HudData::default();
    data.requests.push(Request::UsePetAction { slot: 1 });
    let mut app = pet_action_app(data);
    app.update();
    assert!(app.world().resource::<Messages<NetCommand>>().is_empty());
    assert!(
        app.world().resource::<Messages<UseFailed>>().is_empty(),
        "a hotkey with no mate must not show a refusal"
    );
    let mut data = HudData {
        character_id: 47,
        pets: super::super::View {
            owned: super::View {
                current: Some(Pet {
                    spawn: MateSpawn {
                        tl_id: 12,
                        skills: [7, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                        ..default()
                    },
                    object: 201,
                    level: 30,
                    driving: true,
                    attached_to_own: true,
                    ..default()
                }),
                ..default()
            },
            ..default()
        },
        ..default()
    };
    data.requests.push(Request::UsePetAction { slot: 1 });
    let mut app = pet_action_app(data);
    app.update();
    assert!(app.world().resource::<Messages<NetCommand>>().is_empty());
    assert!(
        app.world().resource::<Messages<UseFailed>>().is_empty(),
        "no game tables means no slot to refuse"
    );
}

#[test]
fn refused_pet_action_reports_the_not_now_result() {
    let mut data = casting_pet(5, true);
    data.requests.push(Request::UsePetAction { slot: 1 });
    let mut app = pet_action_app(data);
    app.update();
    let failed: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<UseFailed>>()
        .drain()
        .collect();
    let [
        UseFailed {
            result,
            server: false,
            ..
        },
    ] = &failed[..]
    else {
        panic!("{failed:?}")
    };
    assert_eq!(*result, usable::code::NOT_NOW);
    assert!(app.world().resource::<Messages<NetCommand>>().is_empty());
    let mut data = casting_pet(30, false);
    data.requests.push(Request::UsePetAction { slot: 2 });
    let mut app = pet_action_app(data);
    app.update();
    let failed: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<UseFailed>>()
        .drain()
        .collect();
    let [
        UseFailed {
            result,
            server: false,
            ..
        },
    ] = &failed[..]
    else {
        panic!("{failed:?}")
    };
    assert_eq!(
        *result,
        usable::code::NOT_NOW,
        "live capture 2026-10-28: any slot on foot shows the local 0x2c refusal"
    );
}

#[test]
fn local_modes_use_attackable_and_active_command_duplicate_guards() {
    let mut data = HudData::default();
    assert!(!request_mode(&mut data, 0));
    let mut game = super::super::super::data::GameData::default();
    game.pets.owned.skills.insert(
        10,
        Skill {
            combat: 110,
            first_seat: None,
            level: 1,
            target_type: 0,
        },
    );
    assert!(data.game.0.set(game).is_ok());
    data.pets.owned.current = Some(Pet {
        spawn: MateSpawn {
            tl_id: 12,
            user_state: 1,
            skills: [10, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ..default()
        },
        object: 201,
        ..default()
    });
    assert!(request_mode(&mut data, 2));
    assert!(!request_mode(&mut data, 2));
    data.pets.owned.commanded_skill = 10;
    assert!(request_mode(&mut data, 2), "same mode interrupts an active local command");
    assert_eq!(data.pets.owned.commanded_skill, 0);
    assert!(
        request_mode(&mut data, 1) && request_mode(&mut data, 1),
        "native passive has no duplicate-state guard"
    );
    data.pets.owned.current.as_mut().unwrap().spawn.skills = [0; 10];
    assert!(!request_mode(&mut data, 3));
    assert!(request_mode(&mut data, 0));
    assert!(!request_mode(&mut data, 0));
}

#[test]
fn mounting_needs_a_live_pet_with_a_driver_seat_in_reach_and_getting_off_names_the_seat() {
    let mut data = HudData::default();
    let mut game = super::super::super::data::GameData::default();
    game.pets.owned.seats.insert(139, (true, true));
    assert!(data.game.0.set(game).is_ok());
    assert!(!can_mount(&data), "no pet");
    data.pets.owned.current = Some(Pet { spawn: MateSpawn { tl_id: 12, ..default() }, object: 201, model: 139, ..default() });
    data.pets.owned.pet_pos = Some((Vec3::new(30.0, 0.0, 0.0), 0.0));
    assert!(!can_mount(&data), "26 m away");
    data.pets.owned.pet_pos = Some((Vec3::new(3.0, 0.0, 0.0), 0.0));
    assert!(can_mount(&data));
    data.pets.owned.mounted = true;
    assert!(!can_mount(&data), "already riding");
    data.pets.owned.mounted = false;
    data.pets.owned.current.as_mut().unwrap().model = 1;
    assert!(!can_mount(&data), "no driver's seat");
    data.pets.owned.current.as_mut().unwrap().model = 139;
    mount(&mut data);
    let sent: Vec<MateCommand> = data.pets.owned.requests.drain(..).map(|r| r.2).collect();
    assert_eq!(sent, [MateCommand::Mount { tl_id: 12, point: 1, reason: 1 }]);
    assert!(!can_mount(&data), "two seconds between requests");
    mount(&mut data);
    assert!(data.pets.owned.requests.is_empty());
    // on the pet: off it from the driver's seat, or off and away
    data.pets.owned.current.as_mut().unwrap().attached_to_own = true;
    data.pets.owned.current.as_mut().unwrap().driving = true;
    old_ride(&mut data);
    unmount(&mut data, false);
    let sent: Vec<MateCommand> = data.pets.owned.requests.drain(..).map(|r| r.2).collect();
    assert_eq!(sent, [MateCommand::Unmount { tl_id: 12, point: 1, reason: 9 }]);
    old_ride(&mut data);
    unmount(&mut data, true);
    let sent: Vec<MateCommand> = data.pets.owned.requests.drain(..).map(|r| r.2).collect();
    assert_eq!(sent, [MateCommand::Unmount { tl_id: 12, point: 1, reason: 10 }]);
    assert!(data.pets.owned.pending_dismiss.is_some(), "the removal waits for the detach");
    let now = web_time::Instant::now();
    assert!(pending_dismiss(&mut data, now).is_none(), "still attached");
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    assert!(pending_dismiss(&mut data, now).is_none(), "the 1000 ms controller starts at the detach");
    assert_eq!(pending_dismiss(&mut data, now + std::time::Duration::from_millis(1001)), Some(MateCommand::Dismiss { tl_id: 12 }));
    assert!(data.pets.owned.pending_dismiss.is_none());
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    old_ride(&mut data);
    unmount(&mut data, false);
    assert!(data.pets.owned.requests.is_empty(), "not riding: nothing to get off");
}

#[test]
#[ignore = "isolated original pet frame/bar, native getter types and physical controls"]
fn original_pet_frame_passive_rename_and_stale_request_identity() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path, player, units, mates) = super::super::active_fixture::runtime(1920.0, 1080.0)?;
    bridge.borrow_mut().pets.owned.current.as_mut().unwrap().level = 15;
    super::push(&rt, &bridge);
    rt.lua
        .load("local s=X2Mate:GetMountSkill(1);assert(s.learned==false and s.learnLevel==20);SetPetBarSkillTooltip(1,PetActionButton1)")
        .exec()?;
    bridge.borrow_mut().pets.owned.current.as_mut().unwrap().level = 30;
    super::push(&rt, &bridge);
    rt.lua
        .load(
            r#"
        local p,total,start,need=X2:GetPetExpToNextLevel()
        assert(type(p)=='number' and total==426750 and start==385700 and need==42050)
        local s=X2Mate:GetMountSkill(1)
        assert(s and type(s.learned)=='boolean' and s.learned==true)
        assert(type(s.skillUsable)=='number' and s.skillUsable==0 and s.needdiver==true)
        assert(X2Mate:GetMountSkill(0)==nil and X2Mate:GetMountSkill(99)==nil)
        local request=X2DialogManager.RequestDefaultDialog
        X2DialogManager.RequestDefaultDialog=function(self,handler,owner)
          local window=request(self,handler,owner); PET_RENAME_DIALOG=window;return window
        end
        ShowChangeMateName('playerpet')
        assert(PET_RENAME_DIALOG and PET_RENAME_DIALOG.editbox:GetText()=='Stormmane')
        PET_RENAME_DIALOG.editbox:SetText('a_1B')
        assert(PET_RENAME_DIALOG.editbox:GetText()=='A_1b' and PET_RENAME_DIALOG.btnOk:IsEnabled())
        PET_RENAME_DIALOG.editbox:SetText('a'..string.rep('B',30))
        assert(PET_RENAME_DIALOG.editbox:MaxTextLength()==26)
        assert(PET_RENAME_DIALOG.editbox:GetText()=='A'..string.rep('b',30) and not PET_RENAME_DIALOG.btnOk:IsEnabled())
        PET_RENAME_DIALOG.editbox:SetText('a B')
        assert(PET_RENAME_DIALOG.editbox:GetText()=='A b' and not PET_RENAME_DIALOG.btnOk:IsEnabled())
        PET_RENAME_DIALOG.editbox:SetText('Thunder')
        assert(PET_RENAME_DIALOG.btnOk:IsEnabled())
        X2Unit:TargetUnit('playerpet')
    "#,
        )
        .exec()?;
    assert!(matches!(
        bridge.borrow().requests.last(),
        Some(super::super::super::api::Request::Target { obj_id: 201 })
    ));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge.clone()))
        .insert_resource(player)
        .insert_resource(units)
        .insert_resource(mates)
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, super::super::update);
    let drain = |app: &mut App| app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|v| v.0).collect::<Vec<_>>();
    super::super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "PET_RENAME_DIALOG.btnCancel")?;
    app.update();
    assert!(drain(&mut app).is_empty());
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("ShowChangeMateName('playerpet');PET_RENAME_DIALOG.editbox:SetText('Thunder')")
        .exec()?;
    super::super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "PET_RENAME_DIALOG.btnOk")?;
    app.update();
    assert!(matches!(drain(&mut app).as_slice(),[Command::Mate(MateCommand::Rename {tl_id:12,name})] if name=="Thunder"));
    assert_eq!(app.world().resource::<Units>().get(201).unwrap().name, "Stormmane", "name waits for receipt");
    // Complete the original dialog's close animation before the next outside interaction.
    aa_ui::lua::tick(&app.world().non_send::<UiRuntime>().lua, 300.0);
    super::super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "petBar.stateBar.buttons[PET_STATE_PASSIVE]")?;
    app.update();
    assert!(matches!(
        drain(&mut app).as_slice(),
        [Command::Mate(MateCommand::UserState { tl_id: 12, state: 1 })]
    ));
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(X2Mate:PetState()=='passive'); X2Mate:SetMateName('Stale')")
        .exec()?;
    app.world_mut().resource_mut::<Units>().by_id.get_mut(&201).unwrap().kind = UnitKind::Npc { template_id: 504, owner_id: 0 };
    app.update();
    assert!(drain(&mut app).is_empty(), "queued name cannot retarget a reused runtime ID");
    {
        let mut data = bridge.borrow_mut();
        super::refresh(app.world(), &mut data);
    }
    super::push(app.world().non_send::<UiRuntime>(), &bridge);
    aa_ui::lua::tick(&app.world().non_send::<UiRuntime>().lua, 0.0);
    app.world().non_send::<UiRuntime>().lua.load("assert(not X2Mate:IsPlayerPetExists() and X2Mate:PetState()==nil); assert(select('#',X2:GetPetExpToNextLevel())==0); assert(petFrame==nil and petBar==nil)").exec()?;
    assert!(app.world().non_send::<UiRuntime>().errors.is_empty());
    drop(app);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn original_chat_and_portal_name_dialogs_keep_case_spaces_and_distinct_limits() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path, _, _, _) = super::super::active_fixture::runtime(1280., 720.)?;
    rt.lua.load(r#"
        local request=X2DialogManager.RequestDefaultDialog
        X2DialogManager.RequestDefaultDialog=function(self,handler,owner)
            local wnd=request(self,handler,owner);NAME_DIALOG=wnd;return wnd
        end
        ShowCreateChatTabName(1)
        NAME_DIALOG.editbox:SetText('Raid Room!')
        assert(NAME_DIALOG.editbox:GetText()=='Raid Room!' and NAME_DIALOG.btnOk:IsEnabled())
        NAME_DIALOG.editbox:SetText('aBc DEFGHIJKL')
        assert(NAME_DIALOG.editbox:MaxTextLength()==10)
        assert(NAME_DIALOG.editbox:GetText()=='aBc DEFGHIJKL' and not NAME_DIALOG.btnOk:IsEnabled())
        NAME_DIALOG.editbox:SetText('x');assert(not NAME_DIALOG.btnOk:IsEnabled())
        NAME_DIALOG.editbox:SetText('');NAME_DIALOG.editbox:SetFocus()
    "#).exec()?;
    rt.type_text("aBc DEFGHIJKL");
    rt.lua.load("assert(NAME_DIALOG.editbox:GetText()=='aBc DEFGHI' and NAME_DIALOG.btnOk:IsEnabled())").exec()?;
    super::super::super::expansion::fixture::click(&rt, "NAME_DIALOG.btnCancel")?;
    aa_ui::lua::tick(&rt.lua, 300.);
    rt.lua.load(r#"
        ShowRegisterPotal()
        NAME_DIALOG.editbox:SetText('AB');assert(not NAME_DIALOG.btnOk:IsEnabled())
        NAME_DIALOG.editbox:SetText('Western Coast')
        assert(NAME_DIALOG.editbox:GetText()=='Western Coast' and NAME_DIALOG.btnOk:IsEnabled())
        NAME_DIALOG.editbox:SetText('A'..string.rep('B',40))
        assert(NAME_DIALOG.editbox:MaxTextLength()==32)
        assert(NAME_DIALOG.editbox:GetText()=='A'..string.rep('B',40) and not NAME_DIALOG.btnOk:IsEnabled())
        NAME_DIALOG.editbox:SetText('');NAME_DIALOG.editbox:SetFocus()
    "#).exec()?;
    rt.type_text(&format!("A{}", "B".repeat(40)));
    rt.lua.load("assert(NAME_DIALOG.editbox:GetText()=='A'..string.rep('B',31) and NAME_DIALOG.btnOk:IsEnabled())").exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    assert!(bridge.borrow().pets.owned.requests.is_empty());
    drop(rt);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn riding_pet() -> HudData {
    let mut data = HudData::default();
    let mut game = super::super::super::data::GameData::default();
    game.pets.owned.seats.insert(139, (true, true));
    data.game.0.set(game).ok();
    data.pets.owned.current = Some(Pet {
        spawn: MateSpawn { tl_id: 12, ..default() },
        object: 201,
        model: 139,
        attached_to_own: true,
        driving: true,
        ..default()
    });
    data.pets.owned.pet_pos = Some((Vec3::new(3.0, 0.0, 0.0), 0.0));
    data
}

fn old_ride(data: &mut HudData) {
    data.pets.owned.last_ride = Some(web_time::Instant::now() - std::time::Duration::from_secs(3));
}

fn sent(data: &mut HudData) -> Vec<MateCommand> {
    data.pets.owned.requests.drain(..).map(|r| r.2).collect()
}

#[test]
fn ride_cooldown_blocks_unmount_for_two_seconds() {
    let mut data = riding_pet();
    mount(&mut data);
    data.pets.owned.requests.clear();
    data.pets.owned.last_ride = Some(web_time::Instant::now());
    unmount(&mut data, false);
    assert!(data.pets.owned.requests.is_empty(), "no CSUnMountMate inside the 2 s ride cooldown");
    old_ride(&mut data);
    unmount(&mut data, false);
    assert_eq!(sent(&mut data), [MateCommand::Unmount { tl_id: 12, point: 1, reason: 9 }]);
}

#[test]
fn dismiss_while_mounted_sends_reason_10_and_defers_the_remove() {
    let mut data = riding_pet();
    old_ride(&mut data);
    unmount(&mut data, true);
    assert_eq!(sent(&mut data), [MateCommand::Unmount { tl_id: 12, point: 1, reason: 10 }], "the native client sends only CSUnMountMate 10");
    assert!(data.pets.owned.pending_dismiss.is_some(), "the CSRemoveMate waits for the detach and 1000 ms");
}

#[test]
fn dismiss_while_dismounted_waits_one_second_then_removes() {
    let mut data = riding_pet();
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    old_ride(&mut data);
    let now = web_time::Instant::now();
    unmount(&mut data, true);
    assert!(data.pets.owned.requests.is_empty(), "no CSUnMountMate on foot");
    assert!(pending_dismiss(&mut data, now).is_none());
    assert_eq!(pending_dismiss(&mut data, now + std::time::Duration::from_millis(1001)), Some(MateCommand::Dismiss { tl_id: 12 }));
}

#[test]
fn a_dismiss_that_never_detaches_expires_without_a_remove() {
    let mut data = riding_pet();
    old_ride(&mut data);
    unmount(&mut data, true);
    data.pets.owned.requests.clear();
    let deadline = data.pets.owned.pending_dismiss.as_ref().unwrap().deadline;
    assert!(pending_dismiss(&mut data, deadline).is_none(), "still attached at the deadline");
    assert!(data.pets.owned.pending_dismiss.is_none(), "the unfulfilled dismiss is dropped");
    assert!(pending_dismiss(&mut data, deadline + std::time::Duration::from_secs(1)).is_none(), "no CSRemoveMate later");
}

#[test]
fn a_manual_unmount_cancels_a_pending_dismiss() {
    let mut data = riding_pet();
    old_ride(&mut data);
    unmount(&mut data, true);
    data.pets.owned.requests.clear();
    old_ride(&mut data);
    unmount(&mut data, false);
    assert_eq!(sent(&mut data), [MateCommand::Unmount { tl_id: 12, point: 1, reason: 9 }]);
    assert!(data.pets.owned.pending_dismiss.is_none(), "no CSRemoveMate after the manual unmount");
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    let now = web_time::Instant::now();
    assert!(pending_dismiss(&mut data, now + std::time::Duration::from_millis(1001)).is_none());
}

#[test]
fn a_pending_dismiss_is_dropped_with_the_pet() {
    let mut data = riding_pet();
    old_ride(&mut data);
    unmount(&mut data, true);
    data.pets.owned.current.as_mut().unwrap().object = 202;
    let now = web_time::Instant::now();
    assert!(pending_dismiss(&mut data, now + std::time::Duration::from_millis(1001)).is_none(), "a swapped mate has no pending remove");
    assert!(data.pets.owned.pending_dismiss.is_none());
    let mut data = riding_pet();
    old_ride(&mut data);
    unmount(&mut data, true);
    data.pets.owned.current = None;
    assert!(pending_dismiss(&mut data, now + std::time::Duration::from_millis(1001)).is_none(), "a despawned mate has no pending remove");
    assert!(data.pets.owned.pending_dismiss.is_none());
}

#[test]
fn mount_reach_is_25_m_in_three_dimensions_and_too_far_says_so() {
    let mut data = riding_pet();
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    data.pets.owned.pet_pos = Some((Vec3::new(25.0, 0.0, 0.0), 0.0));
    assert!(can_mount(&data), "25.0 m is still inside the native squared-distance 625");
    assert_eq!(mount(&mut data), None);
    data.pets.owned.requests.clear();
    data.pets.owned.pet_pos = Some((Vec3::new(0.0, 0.0, 25.001), 0.0));
    assert!(!can_mount(&data), "the z offset counts: the native check is 3D");
    assert_eq!(mount(&mut data), Some("Target is too far."));
    assert!(data.pets.owned.requests.is_empty());
}

#[test]
fn ride_cooldown_near_the_pet_is_silent() {
    let mut data = riding_pet();
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    data.pets.owned.last_ride = Some(web_time::Instant::now());
    assert_eq!(mount(&mut data), None, "the native cooldown has no detail word and shows no text");
    assert!(data.pets.owned.requests.is_empty());
}

#[test]
#[ignore = "loads the original HUD from game_pak; run with AA_PAK set"]
fn original_mountmate_says_target_is_too_far() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path, _, _, _) = super::super::active_fixture::runtime(1280.0, 720.0)?;
    {
        let mut data = bridge.borrow_mut();
        data.pos = Vec3::ZERO;
        data.pets.owned.pet_pos = Some((Vec3::new(30.0, 0.0, 0.0), 0.0));
    }
    rt.lua
        .load("CAPTURED={} SYSMSG=function(text) CAPTURED[#CAPTURED+1]=text end")
        .exec()?;
    rt.lua.load("X2Mate:MountMate()").exec()?;
    assert!(bridge.borrow().pets.owned.requests.is_empty());
    assert_eq!(
        rt.lua.load("return CAPTURED[1]").eval::<String>()?,
        "Target is too far."
    );
    drop(rt);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
