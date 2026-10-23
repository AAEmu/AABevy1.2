use super::*;
use aa_net::NetEvent;

#[test]
fn native_lengths_choose_first_limit_and_preserve_utf8_byte_transport_bounds() {
    let conn = rusqlite::Connection::open_with_flags(
        aa_anim::db::DEFAULT_DB,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let tables = Tables::load(&conn).unwrap();
    for (length, grade) in [
        (1, Some(0)),
        (200, Some(0)),
        (201, Some(1)),
        (400, Some(1)),
        (401, Some(2)),
        (800, Some(3)),
        (801, Some(4)),
        (1000, Some(4)),
        (1001, None),
    ] {
        assert_eq!(tables.grade(&"C".repeat(length)), grade);
    }
    assert_eq!(tables.grade(""), None);
    assert_eq!(tables.voice(6177), Some(aa_midi::mml::Voice::melodic(24)));
}

#[test]
fn executed_native_utf8_lengths_and_quality_match() {
    let mut data = HudData::default();
    data.game
        .0
        .set(super::super::data::GameData::load(aa_anim::db::DEFAULT_DB).unwrap())
        .ok()
        .unwrap();
    for line in include_str!("native.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        if fields[0] == "length" {
            let bytes: Vec<_> = fields[1]
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            assert_eq!(
                std::str::from_utf8(&bytes).unwrap().chars().count(),
                fields[2].parse::<usize>().unwrap()
            );
        } else {
            let grade: u8 = fields[1].parse().unwrap();
            let point = fields[2].parse().unwrap();
            let mut received = aa_net::Actabilities::default();
            received.apply(&Event::Actability(aa_net::ActabilityEvent::Snapshot {
                last: true,
                entries: vec![aa_net::ActabilityValue {
                    id: 33,
                    point,
                    step: grade,
                }],
            }));
            let mut world = World::new();
            world.insert_resource(received);
            super::super::crafting::refresh(&world, &mut data);
            assert_eq!(
                quality(&data, grade).unwrap().to_bits(),
                u32::from_str_radix(fields[3], 16).unwrap(),
                "{line}"
            );
            if grade > 0 {
                assert_eq!(quality(&data, grade - 1), Some(1.));
            }
            assert_eq!(quality(&data, grade + 1), None);
        }
    }
}

#[test]
#[ignore = "isolated original world stage, real composition controls and received item fixtures"]
fn original_editor_preview_confirmation_and_close_follow_native_staging()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime()?;
    fixture::open(&rt)?;
    rt.lua.load(r#"
        assert(composition.score:IsVisible() and composition.score.limit==200)
        local infos=X2UserMusic:GetCompositionLimitInfos(); assert(#infos==7 and infos[1].grade==0 and infos[1].compositionLimit==200 and infos[7].compositionLimit==1000)
        assert(not MUSIC_SAVE:IsEnabled() and not MUSIC_SAMPLE:IsEnabled())
        composition.score.titleEdit:SetText('First Song')
        composition.score.scoreEdit:SetText('T120 O5 C D E F')
        assert(MUSIC_SAVE:IsEnabled() and MUSIC_SAMPLE:IsEnabled())
        assert(not X2UserMusic:PreSaveNotes('9007199254740992','Bad item','C'))
        assert(not X2UserMusic:PreSaveNotes('9007199254740993','','C'))
        assert(not X2UserMusic:PreSaveNotes('9007199254740993','Bad code','T1 C'))
        assert(not X2UserMusic:PreSaveNotes('9007199254740993','Too long',string.rep('C',201)))
        assert(X2UserMusic:PreSaveNotes('9007199254740993','First Song','C D E F'))
        assert(X2UserMusic:TestPlayNotes('T1 C')==-1)
    "#).exec()?;
    assert!(state.borrow().user_music.staged.is_some());
    assert!(
        state.borrow().user_music.requests.is_empty(),
        "PreSave stages without casting"
    );
    super::super::expansion::fixture::click(&rt, "MUSIC_SAMPLE")?;
    assert!(state.borrow().user_music.preview.is_some());
    let generation = state.borrow().user_music.generation;
    rt.lua
        .load("assert(X2UserMusic:TestPlayNotes('T1 C')==-1)")
        .exec()?;
    assert!(
        state.borrow().user_music.preview.is_some(),
        "a failed preview compile leaves the earlier preview intact"
    );
    assert_eq!(state.borrow().user_music.generation, generation);
    rt.lua.load("composition.score:Show(false)").exec()?;
    assert!(state.borrow().user_music.preview.is_none());
    assert!(state.borrow().user_music.generation > generation);
    assert!(
        state.borrow().user_music.requests.is_empty(),
        "preview and close send no music commands"
    );
    fixture::open(&rt)?;
    rt.lua
        .load(
            r#"
        composition.score.titleEdit:SetText('First Song')
        composition.score.scoreEdit:SetText('C D E F')
        local request=X2DialogManager.RequestDefaultDialog
        function X2DialogManager:RequestDefaultDialog(...)
            MUSIC_CONFIRM=request(self,...); return MUSIC_CONFIRM
        end
    "#,
        )
        .exec()?;
    super::super::expansion::fixture::click(&rt, "MUSIC_SAVE")?;
    assert!(
        state.borrow().user_music.requests.is_empty(),
        "Save opens confirmation before casting"
    );
    super::super::expansion::fixture::click(&rt, "MUSIC_CONFIRM.btnCancel")?;
    assert!(state.borrow().user_music.requests.is_empty());
    super::super::expansion::fixture::click(&rt, "MUSIC_SAVE")?;
    super::super::expansion::fixture::click(&rt, "MUSIC_CONFIRM.btnOk")?;
    rt.lua.load("assert(composition.score==nil)").exec()?;
    let mut data = state.borrow_mut();
    assert_eq!(data.user_music.requests.len(), 1);
    let pending = data.user_music.cast.clone().unwrap();
    assert_eq!(pending.request.source().slot, 3);
    assert_eq!(pending.request.source().item.grade, 2);
    assert_eq!(pending.request.source().item.made_unit_id, 77);
    assert_eq!(pending.request.grade(), 0);
    // Only an actual received own item caster fires the staged upload, once.
    let mut fired = aa_net::skill::SkillFired {
        skill_id: 22215,
        tl: 12,
        caster: aa_net::buff::BuffCaster {
            kind: 2,
            obj_id: 100,
        },
        target: aa_net::skill::SkillTarget::unit(100),
        raw_caster: aa_net::skill::SkillCasterData::Item {
            obj_id: 100,
            item_id: pending.request.source().item.id,
            template_id: 27902,
            type1: 2,
            type2: 77,
        },
        raw_target: aa_net::skill::SkillTargetData::Unit(100),
        delay_ms: 0,
        channeling_ms: 0,
        fire_anim_id: 0,
        result: default(),
        raw_final_flag: 0,
    };
    assert!(
        receive(&mut data, &Event::SkillFired(fired.clone())).is_none(),
        "a stale fire before the cast write cannot upload"
    );
    assert!(
        receive(
            &mut data,
            &Event::MusicComposeRequested {
                character_id: 47,
                request: pending.request.clone()
            }
        )
        .is_none()
    );
    fired.raw_caster = aa_net::skill::SkillCasterData::Unit(100);
    assert!(receive(&mut data, &Event::SkillFired(fired.clone())).is_none());
    fired.raw_caster = aa_net::skill::SkillCasterData::Item {
        obj_id: 101,
        item_id: pending.request.source().item.id,
        template_id: 27902,
        type1: 2,
        type2: 77,
    };
    assert!(receive(&mut data, &Event::SkillFired(fired.clone())).is_none());
    fired.raw_caster = aa_net::skill::SkillCasterData::Item {
        obj_id: 100,
        item_id: pending.request.source().item.id,
        template_id: 27902,
        type1: 2,
        type2: 77,
    };
    if let aa_net::skill::SkillCasterData::Item { type2, .. } = &mut fired.raw_caster {
        *type2 = 78;
    }
    assert!(receive(&mut data, &Event::SkillFired(fired.clone())).is_none());
    if let aa_net::skill::SkillCasterData::Item { type2, .. } = &mut fired.raw_caster {
        *type2 = 77;
    }
    assert!(matches!(
        receive(&mut data, &Event::SkillFired(fired.clone())),
        Some(Command::Music {
            character_id: 47,
            command: MusicCommand::UploadNotes {
                slot: 3,
                slot_type: 2,
                ..
            }
        })
    ));
    assert!(receive(&mut data, &Event::SkillFired(fired.clone())).is_none());
    let mut sent = pending.clone();
    sent.sent = true;
    data.user_music.cast = Some(sent.clone());
    fired.result.code = 1;
    assert!(receive(&mut data, &Event::SkillFired(fired.clone())).is_none());
    assert!(data.user_music.cast.is_none(), "failed fire never uploads");
    fired.result.code = 0;
    sent.timeline = Some(99);
    data.user_music.cast = Some(sent.clone());
    assert!(receive(&mut data, &Event::SkillFired(fired.clone())).is_none());
    assert!(
        data.user_music.cast.is_some(),
        "different timeline cannot complete staged save"
    );
    receive(
        &mut data,
        &Event::CastingStopped {
            tl: 99,
            duration_ms: 0,
        },
    );
    assert!(data.user_music.cast.is_none());
    data.user_music.cast = Some(sent.clone());
    receive(
        &mut data,
        &Event::MusicComposeRejected {
            character_id: 47,
            request: sent.request.clone(),
            message: "socket refused".into(),
        },
    );
    assert!(data.user_music.cast.is_none());
    data.user_music.cast = Some(sent.clone());
    data.character_id = 48;
    assert!(receive(&mut data, &Event::SkillFired(fired.clone())).is_none());
    assert!(
        data.user_music.cast.is_none(),
        "previous character cannot upload"
    );
    data.character_id = 47;
    data.snapshot.unit_id = "101".into();
    data.user_music.cast = Some(sent);
    assert!(receive(&mut data, &Event::SkillFired(fired)).is_none());
    assert!(
        data.user_music.cast.is_none(),
        "same-character relog with a new actor cannot upload an old cast"
    );
    drop(data);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn native_compiler_input_strips_only_initial_bom_and_limits_unicode_scalars() {
    assert_eq!(compiler_input("\u{feff}C"), "C");
    assert_eq!(compiler_input("C\u{feff}"), "C\u{feff}");
    let notes = format!("\u{feff}{}Z", "\u{1f600}".repeat(3000));
    let input = compiler_input(&notes);
    assert_eq!(input.chars().count(), 3000);
    assert_eq!(input.len(), 12000);
    assert!(!input.ends_with('Z'));
    assert_eq!(compiler_input(&"C".repeat(3001)).len(), 3000);
}

fn started(skill_id: u32) -> aa_net::skill::SkillStarted {
    aa_net::skill::SkillStarted {
        skill_id,
        tl: 1,
        caster: aa_net::buff::BuffCaster {
            kind: 0,
            obj_id: 100,
        },
        target: aa_net::skill::SkillTarget::unit(100),
        raw_caster: aa_net::skill::SkillCasterData::Unit(100),
        raw_target: aa_net::skill::SkillTargetData::Unit(100),
        cast_ms: 0,
        base_cast_ms: 0,
        synergy: 0,
        result: 0,
        result_value: 0,
        raw_result: default(),
    }
}

#[test]
#[ignore = "original UI/item/proficiency fixture for the actual received music action callbacks"]
fn native_started_actions_compile_staged_notes_and_close_without_compose_or_id_hardcoding()
-> Result<(), Box<dyn std::error::Error>> {
    let (_, state, path) = fixture::runtime()?;
    let mut data = state.borrow_mut();
    data.user_music.cache.borrow_mut().notes = "O5 C8".into();
    let tables = &data.game.get().unwrap().user_music;
    assert_eq!(tables.voice(34598), Some(aa_midi::mml::Voice::melodic(106)));
    assert_eq!(tables.voice(34599), Some(aa_midi::mml::Voice::melodic(73)));
    assert_eq!(tables.voice(6183), None);
    assert_eq!(tables.voice(14047), None);
    assert_eq!(tables.actions(22213), [93]);
    assert_eq!(tables.actions(22214), [94]);
    assert_eq!(tables.actions(22217), [94]);
    assert!(tables.actions(22215).is_empty());
    let mut event = started(22213);
    for caster in [
        aa_net::skill::SkillCasterData::Unit(100),
        aa_net::skill::SkillCasterData::Item {
            obj_id: 100,
            item_id: 1,
            template_id: 6177,
            type1: 0,
            type2: 0,
        },
        aa_net::skill::SkillCasterData::Mount {
            obj_id: 100,
            mount_skill_id: 1,
        },
    ] {
        event.raw_caster = caster;
        let commands = started_commands(&mut data, &event);
        let [
            Command::Music {
                character_id: 47,
                command: MusicCommand::CacheMidi { midi },
            },
        ] = commands.as_slice()
        else {
            panic!("{commands:?}");
        };
        assert!(aa_midi::smf::parse(midi).is_ok());
    }
    for caster in [
        aa_net::skill::SkillCasterData::Unk1(100),
        aa_net::skill::SkillCasterData::Doodad(100),
        aa_net::skill::SkillCasterData::Unit(101),
    ] {
        event.raw_caster = caster;
        assert!(started_commands(&mut data, &event).is_empty());
    }
    event.raw_caster = aa_net::skill::SkillCasterData::Unit(100);
    event.raw_result.code = 1;
    assert!(started_commands(&mut data, &event).is_empty());
    event.raw_result.code = 0;
    event.tl = 0;
    assert!(started_commands(&mut data, &event).is_empty());
    for id in [22214, 22217] {
        assert!(matches!(
            started_commands(&mut data, &started(id)).as_slice(),
            [Command::Music {
                character_id: 47,
                command: MusicCommand::End
            }]
        ));
    }
    assert!(started_commands(&mut data, &started(22215)).is_empty());
    data.user_music.cache.borrow_mut().notes = "T1 C".into();
    assert!(started_commands(&mut data, &started(22213)).is_empty());
    assert!(
        data.user_music.cast.is_none(),
        "normal play never composes or uploads"
    );
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn catalog_attachment_is_independent_of_application_target_method() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE music_note_limits(step,note_length); CREATE TABLE items(id,category_id); CREATE TABLE instrument_sounds(item_id,midi); CREATE TABLE skill_effects(id,skill_id,effect_id,application_method_id); CREATE TABLE effects(id,actual_type,actual_id); CREATE TABLE special_effects(id,special_effect_type_id); INSERT INTO effects VALUES(1,'SpecialEffect',10); INSERT INTO special_effects VALUES(10,94);").unwrap();
    for method in 0..6 {
        conn.execute(
            "INSERT INTO skill_effects VALUES(?1,?2,1,?1)",
            [method, 90000 + method],
        )
        .unwrap();
    }
    let tables = Tables::load(&conn).unwrap();
    for method in 0..6 {
        assert_eq!(tables.actions(90000 + method), [94]);
    }
}

#[test]
#[ignore = "original received proficiency fixture, ordered Bevy message callbacks without a Session"]
fn score_receipt_and_started_callbacks_follow_packet_order_without_automatic_writes()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime()?;
    state.borrow().user_music.cache.borrow_mut().notes = "C".into();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .init_resource::<playback::Playback>()
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .init_resource::<crate::hud::HudBuilt>()
        .add_systems(Update, observe_events.after(crate::hud::enchant::track));
    crate::hud::enchant::plugin(&mut app);
    let receipt = |notes: &str| {
        Event::Notice(aa_net::notice::NoticeEvent::UserNoteLoaded(Some(
            aa_net::notice::UserNote {
                song_id: 42,
                tooltip: false,
                title: "Harbor".into(),
                notes: notes.into(),
            },
        )))
    };
    // Activate the world cache without changing the already established HUD actor.
    for event in [
        Event::ReadyToConnect {
            world_id: 17,
            cookie: aa_net::login::WorldCookie {
                cookie: 0,
                addr: "127.0.0.1:1".parse().unwrap(),
            },
        },
        Event::PlayerSpawn(Box::new(crate::hud::expansion::fixture::spawn())),
    ] {
        state.borrow().user_music.cache.borrow_mut().apply(&event);
    }
    state.borrow().user_music.cache.borrow_mut().notes = "C".into();
    app.world_mut()
        .write_message(NetEvent(Event::SkillStarted(started(22213))));
    app.world_mut().write_message(NetEvent(receipt("T1 C")));
    app.update();
    let commands: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .collect();
    assert!(
        matches!(
            commands.as_slice(),
            [NetCommand(Command::Music {
                command: MusicCommand::CacheMidi { .. },
                ..
            })]
        ),
        "{commands:?}"
    );
    app.world_mut()
        .write_message(NetEvent(Event::SkillStarted(started(22213))));
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .count(),
        0
    );
    app.world_mut().write_message(NetEvent(receipt("CDE")));
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .count(),
        0,
        "receipt sends no command"
    );
    app.world_mut()
        .write_message(NetEvent(Event::SkillStarted(started(22217))));
    app.update();
    let commands: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .collect();
    assert!(matches!(
        commands.as_slice(),
        [NetCommand(Command::Music {
            command: MusicCommand::End,
            ..
        })]
    ));
    // Equipment callbacks compile inside the same production receipt dispatcher.
    // The HUD/crafting snapshots can already contain future frame-final values.
    for (first, template, expected) in [
        (false, 34598, 24),
        (false, 6177, 106),
        (true, 34598, 106),
        (true, 6177, 24),
    ] {
        let equip = Event::EquipmentChanged {
            obj_id: 100,
            items: vec![(
                aa_net::model::slot::MUSICAL as u8,
                Some(aa_net::model::Item {
                    id: 200,
                    template_id: template,
                    count: 1,
                    ..default()
                }),
            )],
        };
        let start = Event::SkillStarted(started(22213));
        for event in if first {
            [equip, start]
        } else {
            [start, equip]
        } {
            app.world_mut().write_message(NetEvent(event));
        }
        app.update();
        let commands: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .collect();
        let [
            NetCommand(Command::Music {
                command: MusicCommand::CacheMidi { midi },
                ..
            }),
        ] = commands.as_slice()
        else {
            panic!("{commands:?}");
        };
        let song = aa_midi::smf::parse(midi)?;
        assert!(
            song.events
                .iter()
                .any(|e| e.kind == aa_midi::smf::Kind::ProgramChange { program: expected }),
            "{song:?}"
        );
    }
    state.borrow().user_music.cache.borrow_mut().notes = "C".repeat(201);
    let tier = |step| {
        Event::Actability(aa_net::ActabilityEvent::Snapshot {
            last: true,
            entries: vec![aa_net::ActabilityValue {
                id: 33,
                point: 10000,
                step,
            }],
        })
    };
    app.world_mut()
        .write_message(NetEvent(Event::SkillStarted(started(22213))));
    app.world_mut().write_message(NetEvent(tier(1)));
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .count(),
        0,
        "later proficiency must not admit an earlier cast"
    );
    app.world_mut()
        .write_message(NetEvent(Event::SkillStarted(started(22213))));
    app.world_mut().write_message(NetEvent(tier(0)));
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .count(),
        1,
        "later downgrade must not suppress an earlier cast"
    );
    app.world_mut().write_message(NetEvent(tier(2)));
    app.update();
    app.world_mut().write_message(NetEvent(Event::Actability(
        aa_net::ActabilityEvent::TierChanged {
            is_upgrade: false,
            id: 33,
            step: 1,
        },
    )));
    app.world_mut()
        .write_message(NetEvent(Event::SkillStarted(started(22213))));
    app.update();
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .count(),
        1,
        "downgrade ACK resolves points before compiling"
    );
    let point = state
        .borrow()
        .user_music
        .actabilities
        .get(33)
        .unwrap()
        .point;
    assert_eq!(
        point,
        crate::hud::crafting::music_upper_limit(&state.borrow(), 1)
    );
    // Real receipt -> Started -> MIDI covers the compiler wrapper's full converted capacity.
    app.world_mut().write_message(NetEvent(tier(4)));
    for (notes, tracks) in [
        (format!("C{}", " ".repeat(999)), 1),
        (format!("C{},D", " ".repeat(998)), 2),
        (format!("C{},", " ".repeat(998)).repeat(3), 3),
    ] {
        app.world_mut().write_message(NetEvent(receipt(&notes)));
        app.world_mut()
            .write_message(NetEvent(Event::SkillStarted(started(22213))));
        app.update();
        let commands: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .collect();
        let [
            NetCommand(Command::Music {
                command: MusicCommand::CacheMidi { midi },
                ..
            }),
        ] = commands.as_slice()
        else {
            panic!("{} chars: {commands:?}", notes.chars().count());
        };
        assert_eq!(aa_midi::smf::parse(midi)?.tracks, tracks);
    }
    let directory = path.with_extension("music");
    if directory.exists() {
        std::fs::remove_dir_all(directory)?;
    }
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
#[ignore = "real device and original DLS, received own/foreign MIDI through the application controller"]
fn received_performances_admit_pcm_follow_native_end_and_survive_foreign_registry_loss()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime()?;
    let engine = aa_audio::Engine::start(aa_audio::Settings {
        audible: true,
        ..default()
    });
    engine.set_volumes(aa_audio::Volumes {
        master: 0.,
        ..default()
    });
    let mut units = aa_net::state::Units::default();
    let mut foreign = crate::hud::expansion::fixture::spawn().unit;
    foreign.obj_id = 200;
    units.by_id.insert(200, foreign.clone());
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(HudBridge(state))
        .insert_non_send(rt)
        .insert_resource(engine.clone())
        .insert_resource(units)
        .init_resource::<playback::Playback>()
        .init_resource::<crate::hud::HudBuilt>()
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_message::<MusicItemUsed>()
        .add_systems(
            Update,
            (observe_events.after(crate::hud::enchant::track), update).chain(),
        );
    crate::hud::enchant::plugin(&mut app);
    let performance = |actor, notes| {
        Event::Notice(aa_net::notice::NoticeEvent::UserMusic {
            obj_id: actor,
            author: "Bard".into(),
            midi: aa_midi::mml::compile(notes, aa_midi::mml::Voice::melodic(73), 1., 0).unwrap(),
        })
    };
    app.world_mut()
        .write_message(NetEvent(performance(100, "C64")));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    let mut ends = 0;
    while ends == 0 && std::time::Instant::now() < deadline {
        app.update();
        let _ = engine.take_finished();
        for command in app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
        {
            assert!(
                matches!(
                    command,
                    NetCommand(Command::Music {
                        character_id: 47,
                        command: MusicCommand::End
                    })
                ),
                "{command:?}"
            );
            ends += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(engine.stats().device_error, None);
    assert_eq!(ends, 1, "own actual received performance ends once");
    app.world_mut()
        .resource_mut::<aa_net::state::Units>()
        .by_id
        .clear();
    app.world_mut()
        .write_message(NetEvent(Event::UnitSpawned(Box::new(foreign))));
    app.world_mut()
        .write_message(NetEvent(performance(200, "C2")));
    app.world_mut()
        .write_message(NetEvent(Event::UnitsRemoved(vec![200])));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    while engine.stats().started < 2 && std::time::Instant::now() < deadline {
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(engine.stats().started, 2);
    app.world_mut()
        .resource_mut::<aa_net::state::Units>()
        .by_id
        .remove(&200);
    app.world_mut()
        .write_message(NetEvent(Event::UnitsRemoved(vec![200])));
    let hold = std::time::Instant::now() + std::time::Duration::from_millis(100);
    while std::time::Instant::now() < hold {
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        engine.stats().active_voices,
        1,
        "foreign registry disappearance retains the admitted voice"
    );
    while engine.stats().pcm_submissions != 0 && std::time::Instant::now() < deadline {
        app.update();
        let _ = engine.take_finished();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.update();
    assert_eq!(engine.stats().pcm_submissions, 0);
    assert_eq!(
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .count(),
        0,
        "foreign completion never sends End"
    );
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
