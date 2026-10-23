use super::*;

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
