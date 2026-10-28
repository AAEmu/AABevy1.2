use super::*;

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
    data.pets.owned.current = Some(Pet {
        spawn: MateSpawn {
            tl_id: 12,
            ..default()
        },
        object: 201,
        model: 139,
        ..default()
    });
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
    assert_eq!(
        sent,
        [MateCommand::Mount {
            tl_id: 12,
            point: 1,
            reason: 1
        }]
    );
    assert!(!can_mount(&data), "two seconds between requests");
    mount(&mut data);
    assert!(data.pets.owned.requests.is_empty());
    // on the pet: off it from the driver's seat, or off and away
    data.pets.owned.current.as_mut().unwrap().attached_to_own = true;
    data.pets.owned.current.as_mut().unwrap().driving = true;
    old_ride(&mut data);
    unmount(&mut data, false);
    let sent: Vec<MateCommand> = data.pets.owned.requests.drain(..).map(|r| r.2).collect();
    assert_eq!(
        sent,
        [MateCommand::Unmount {
            tl_id: 12,
            point: 1,
            reason: 9
        }]
    );
    old_ride(&mut data);
    unmount(&mut data, true);
    let sent: Vec<MateCommand> = data.pets.owned.requests.drain(..).map(|r| r.2).collect();
    assert_eq!(
        sent,
        [MateCommand::Unmount {
            tl_id: 12,
            point: 1,
            reason: 10
        }]
    );
    assert!(
        data.pets.owned.pending_dismiss.is_some(),
        "the removal waits for the detach"
    );
    let now = web_time::Instant::now();
    assert!(pending_dismiss(&mut data, now).is_none(), "still attached");
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    assert!(
        pending_dismiss(&mut data, now).is_none(),
        "the 1000 ms controller starts at the detach"
    );
    assert_eq!(
        pending_dismiss(&mut data, now + std::time::Duration::from_millis(1001)),
        Some(MateCommand::Dismiss { tl_id: 12 })
    );
    assert!(data.pets.owned.pending_dismiss.is_none());
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    old_ride(&mut data);
    unmount(&mut data, false);
    assert!(
        data.pets.owned.requests.is_empty(),
        "not riding: nothing to get off"
    );
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
        spawn: MateSpawn {
            tl_id: 12,
            ..default()
        },
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

#[test]
fn ride_cooldown_blocks_unmount_for_two_seconds() {
    let mut data = riding_pet();
    mount(&mut data);
    data.pets.owned.requests.clear();
    data.pets.owned.last_ride = Some(web_time::Instant::now());
    unmount(&mut data, false);
    assert!(
        data.pets.owned.requests.is_empty(),
        "no CSUnMountMate inside the 2 s ride cooldown"
    );
    old_ride(&mut data);
    unmount(&mut data, false);
    let sent: Vec<MateCommand> = data.pets.owned.requests.drain(..).map(|r| r.2).collect();
    assert_eq!(
        sent,
        [MateCommand::Unmount {
            tl_id: 12,
            point: 1,
            reason: 9
        }]
    );
}

#[test]
fn dismiss_while_mounted_sends_reason_10_and_defers_the_remove() {
    let mut data = riding_pet();
    old_ride(&mut data);
    unmount(&mut data, true);
    let sent: Vec<MateCommand> = data.pets.owned.requests.drain(..).map(|r| r.2).collect();
    assert_eq!(
        sent,
        [MateCommand::Unmount {
            tl_id: 12,
            point: 1,
            reason: 10
        }],
        "the native client sends only CSUnMountMate 10"
    );
    assert!(
        data.pets.owned.pending_dismiss.is_some(),
        "the CSRemoveMate waits for the detach and 1000 ms"
    );
}

#[test]
fn dismiss_while_dismounted_waits_one_second_then_removes() {
    let mut data = riding_pet();
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    old_ride(&mut data);
    let now = web_time::Instant::now();
    unmount(&mut data, true);
    assert!(
        data.pets.owned.requests.is_empty(),
        "no CSUnMountMate on foot"
    );
    assert!(pending_dismiss(&mut data, now).is_none());
    assert_eq!(
        pending_dismiss(&mut data, now + std::time::Duration::from_millis(1001)),
        Some(MateCommand::Dismiss { tl_id: 12 })
    );
}

#[test]
fn mount_reach_is_25_m_in_three_dimensions_and_too_far_says_so() {
    let mut data = riding_pet();
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    data.pets.owned.pet_pos = Some((Vec3::new(25.0, 0.0, 0.0), 0.0));
    assert!(
        can_mount(&data),
        "25.0 m is still inside the native squared-distance 625"
    );
    assert_eq!(mount(&mut data), None);
    data.pets.owned.requests.clear();
    data.pets.owned.pet_pos = Some((Vec3::new(0.0, 0.0, 25.001), 0.0));
    assert!(
        !can_mount(&data),
        "the z offset counts: the native check is 3D"
    );
    assert_eq!(mount(&mut data), Some("Target is too far."));
    assert!(data.pets.owned.requests.is_empty());
}

#[test]
fn ride_cooldown_near_the_pet_is_silent() {
    let mut data = riding_pet();
    data.pets.owned.current.as_mut().unwrap().attached_to_own = false;
    data.pets.owned.last_ride = Some(web_time::Instant::now());
    assert_eq!(
        mount(&mut data),
        None,
        "the native cooldown has no detail word and shows no text"
    );
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
