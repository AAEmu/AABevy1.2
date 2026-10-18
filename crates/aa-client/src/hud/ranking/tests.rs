use super::*;
use aa_net::{RankCharacterRow, RankSnapshot, RankSnapshotEntry, RankSnapshotScope};
use std::{cell::RefCell, rc::Rc};

fn setup() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", 1920., 1080.)?;
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let state = Rc::new(RefCell::new(HudData::default()));
    assert!(
        state
            .borrow()
            .game
            .0
            .set(super::super::data::GameData::load(aa_anim::db::DEFAULT_DB)?)
            .is_ok()
    );
    super::super::api::install(&mut rt, &state)?;
    state.borrow_mut().character_id = 47;
    let mut selected = aa_net::model::CharacterState::default();
    selected.character.id = 47;
    dispatch(Some(&rt.lua), &state, &Event::CharacterSelected(Box::new(selected)));
    dispatch(
        Some(&rt.lua),
        &state,
        &Event::PlayerSpawn(Box::new(super::super::expansion::fixture::spawn())),
    );
    Ok((rt, state))
}
fn personal(rank_id: u32, v1: i32) -> RankCharacterRow {
    RankCharacterRow {
        account_id: 1,
        account: b"account".to_vec(),
        character_id: 47,
        rank_id,
        world_id: 1,
        v1,
        raw_sum_v1: 0,
        v2: 16777217,
        raw_sum_v2: 1,
        recorded_raw: 1_700_000_000,
    }
}
fn send(rt: Option<&UiRuntime>, state: &Shared, event: RankingEvent) {
    dispatch(rt.map(|rt| &rt.lua), state, &Event::Ranking(event));
}
fn records(rt: Option<&UiRuntime>, state: &Shared, ack: u64, rows: Vec<RankCharacterRow>) {
    send(
        rt,
        state,
        RankingEvent::Personal {
            type_id: 999,
            error: 0,
            ack_time_raw: ack,
            count: rows.len() as i32,
            rows,
        },
    );
}
fn entry(character_id: u32, ranking: i32, v1: i32) -> RankSnapshotEntry {
    RankSnapshotEntry {
        character_id,
        rank_id: 999,
        world_id: 1,
        v1,
        v2: 16777217,
        updated_raw: 1_700_000_000,
        ranking,
    }
}
fn snapshot(rt: Option<&UiRuntime>, state: &Shared, rank_id: u32, updated: u64, entries: Vec<RankSnapshotEntry>) {
    send(
        rt,
        state,
        RankingEvent::Snapshot {
            rank_id,
            error: 0,
            board: RankSnapshot {
                board_id: 77,
                updated_raw: updated,
                reset_raw: 1_699_999_000,
                entry_count: entries.len() as i32,
                entries,
                scope_count: 2,
                scopes: vec![
                    RankSnapshotScope {
                        scope_id: 7,
                        v1: -16777217,
                        v2: 4,
                    },
                    RankSnapshotScope { scope_id: 7, v1: 2, v2: 3 },
                ],
            },
        },
    );
}
#[derive(Debug, PartialEq)]
enum Request {
    Ranking(u32, RankingCommand),
    Name(u32, u32),
}
fn commands(state: &Shared) -> Vec<Request> {
    flush(&mut state.borrow_mut())
        .into_iter()
        .map(|command| match command {
            aa_net::Command::Ranking { character_id, command } => Request::Ranking(character_id, command),
            aa_net::Command::QueryCharacterName {
                character_id,
                requested_id,
            } => Request::Name(character_id, requested_id),
            other => panic!("unexpected command {other:?}"),
        })
        .collect()
}
fn ranking_command(rank_id: Option<u32>) -> Request {
    Request::Ranking(
        47,
        rank_id.map_or(RankingCommand::RequestPlayerRecords, |rank_id| RankingCommand::RequestSnapshots {
            rank_id,
        }),
    )
}
fn name_reply(state: &Shared, id: u32, name: &[u8]) {
    dispatch(
        None,
        state,
        &Event::CharacterName(aa_net::CharacterName {
            character_id: id,
            name: name.to_vec(),
        }),
    );
}
fn fake_calendar(rt: &UiRuntime) -> mlua::Result<()> {
    // Same explicitly mocked localtime provider as the executed original numeric/table oracle.
    rt.lua
        .load("os.date=function() return {year=2006,month=5,day=4,hour=3,min=22,sec=11,wday=5} end")
        .exec()
}

#[test]
fn original_catalog_labels_child_order_and_strict_numeric_getter_arity() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua
        .load(
            r#"
      assert(X2Rank:GetRankCount()==4 and X2Rank:GetDefaultTabIndex()==0 and X2Rank:GetSelectedTabIndex()==0)
      local names={'Arena Ranks','Sport Fisherman','Heavyweight Fish','Auto Racing'}
      for i=1,4 do local info=X2Rank:GetRankInfo(i);assert(info.name==names[i] and type(info.v1)=='string' and type(info.v2)=='string') end
      for _,v in ipairs({0,-1,5,'1',true,0/0,math.huge}) do assert(select('#',X2Rank:GetRankInfo(v))==0) end
      assert(select('#',X2Rank:GetRankInfo())==0)
      assert(select('#',X2Rank:IsOnGoingRank('1'))==0)
      for _,v in ipairs({0,-1,1,4,5,0/0,math.huge}) do assert(select('#',X2Rank:IsOnGoingRank(v))==1 and X2Rank:IsOnGoingRank(v)==false) end
      assert(select('#',X2Rank:IsActivatedPeriod())==1 and X2Rank:IsActivatedPeriod()==false)
    "#,
        )
        .exec()?;
    let data = state.borrow();
    let tables = &data.game.get().unwrap().ranking;
    assert_eq!(tables.rows.iter().map(|entry| entry.id).collect::<Vec<_>>(), [1, 2, 3, 4]);
    for (index, entry) in tables.rows.iter().enumerate() {
        assert_eq!(
            entry.scope_ids,
            ((index as u32 * 10 + 1)..=(index as u32 * 10 + 10)).collect::<Vec<_>>()
        );
        assert_eq!(
            entry.reward_links.iter().map(|&(scope, _)| scope).collect::<Vec<_>>(),
            entry.scope_ids
        );
    }
    assert_eq!(
        tables.rows[0].reward_links,
        [
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 24),
            (5, 26),
            (6, 25),
            (7, 28),
            (8, 27),
            (9, 29),
            (10, 30)
        ]
    );
    assert_eq!(tables.rows[0].v1, "Score");
    assert_eq!(tables.rows[0].v2, "Wins");
    Ok(())
}

#[test]
fn executed_native_getter_shapes_keep_sparse_keys_duplicates_names_raw_values_and_cached_times() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    fake_calendar(&rt)?;
    records(Some(&rt), &state, 1_700_000_000, vec![personal(1, -16777217)]);
    rt.lua.load("assert(next(X2Rank:GetSnapshot())==nil);local p=X2Rank:GetPlayerRecord();assert(p.v1==-16777216 and p.v2==16777216 and p.year==2006 and p.minute==22)").exec()?;
    name_reply(&state, 42, b"Actual name\0unused");
    let world = aa_net::login::World {
        id: 1,
        name: "Actual world\0unused".into(),
        online: true,
        congestion: 0,
        race_congestion: [0; 9],
    };
    dispatch(
        Some(&rt.lua),
        &state,
        &Event::WorldList(aa_net::login::WorldList {
            worlds: vec![world],
            characters: vec![],
        }),
    );
    let mut missing = entry(43, 2, -9);
    missing.world_id = 255;
    snapshot(
        Some(&rt),
        &state,
        1,
        10_000,
        vec![entry(42, 1, 11), missing, entry(42, 1, 22), entry(42, -3, 44)],
    );
    assert_eq!(state.borrow().ranking.live.next_update[&1], 13600);
    rt.lua.load(r#"
      local s=X2Rank:GetSnapshot();assert(s[1].v1==22 and s[-3].v1==44 and s[2].v1==-9)
      assert(s[1].name=='Actual name' and s[1].worldName=='Actual world')
      assert(s[2].name==nil and s[2].worldName==nil and s[2].v2==16777216)
      assert(s[2].year==2006 and s[2].month==5 and s[2].day==4 and s[2].hour==3 and s[2].minute==22 and s[2].second==11)
      local c=X2Rank:GetSnapshotScope();assert(#c.snapshotScopes==2 and c.snapshotScopes[1].scope==7 and c.snapshotScopes[2].scope==7)
      assert(c.snapshotScopes[1].v1==-16777216 and #c.rankScopes==10 and c.rankScopes[1].from==1 and c.rankScopes[10].to==3000)
      local reward=X2Rank:GetReward();assert(#reward==10 and reward[1].scopeName=='Mythic' and reward[1].item.type==32488 and reward[1].item.grade==11)
      local t=X2Rank:GetSnapshotUpdatedTime();assert(t.prev.year==2006 and t.next.minute==22)
    "#).exec()?;
    assert_eq!(commands(&state), [Request::Name(47, 43)]);
    snapshot(Some(&rt), &state, 1, 0, vec![]);
    assert_eq!(state.borrow().ranking.live.next_update[&1], 13600);
    assert_eq!(
        state.borrow().ranking.live.received.by_id[&1].snapshot.as_ref().unwrap().reset_raw,
        1_699_999_000
    );
    rt.lua.load("os.date=function() return nil end;assert(select('#',X2Rank:GetPlayerRecord())==0 and select('#',X2Rank:GetSnapshotUpdatedTime())==0 and select('#',X2Rank:GetResetPeriod())==0)").exec()?;
    Ok(())
}

#[test]
fn local_locks_name_pending_precedence_and_readiness_follow_original_frame_order() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    fake_calendar(&rt)?;
    records(Some(&rt), &state, 100, vec![personal(1, 7), personal(2, 8)]);
    let captured = Rc::new(RefCell::new(Vec::new()));
    let output = captured.clone();
    let s = state.clone();
    rt.lua.globals().set(
        "recordRankNotice",
        rt.lua.create_function(move |_, event: String| {
            let data = s.borrow();
            let p = data.ranking.current();
            output.borrow_mut().push((event, p.lock_seconds, p.ready));
            Ok(())
        })?,
    )?;
    rt.lua.load("local w=UIParent:CreateWidget('emptywidget','rankListener','UIParent');for _,e in ipairs({'RANK_LOCK','RANK_UNLOCK','RANK_SNAPSHOTS'}) do w:RegisterEvent(e) end;w:SetHandler('OnEvent',function(self,e) recordRankNotice(e) end)").exec()?;
    state.borrow_mut().ranking.live.lock_seconds = 7.;
    rt.lua.load("X2Rank:RequestPlayerRecords(1);X2Rank:RequestPlayerRecords(4);X2Rank:RequestPlayerRecords('1');X2Rank:RequestPlayerRecords(0/0)").exec()?;
    assert_eq!(commands(&state), [ranking_command(None)]);
    assert_eq!(captured.borrow()[0].0, "RANK_LOCK");
    assert_eq!(captured.borrow()[0].1, 7.);
    state.borrow_mut().ranking.live.lock_seconds = 1.;
    dispatch(
        None,
        &state,
        &Event::CharacterNameRequested {
            character_id: 47,
            requested_id: 47,
        },
    );
    snapshot(Some(&rt), &state, 1, 100, vec![entry(47, 1, 1), entry(42, 2, 2)]);
    assert_eq!(commands(&state), [Request::Name(47, 42)]);
    frame(Some(&rt.lua), &state, 4.);
    assert_eq!(captured.borrow().len(), 1);
    assert!(!state.borrow().ranking.live.ready);
    name_reply(&state, 47, b"");
    name_reply(&state, 42, &[255, 0, b'X']);
    frame(Some(&rt.lua), &state, f32::from_bits(4f32.to_bits() + 1));
    let result = captured.borrow();
    assert_eq!(result.len(), 3);
    assert_eq!(result[1], ("RANK_UNLOCK".into(), 1., false));
    assert_eq!(result[2], ("RANK_SNAPSHOTS".into(), 0., true));
    drop(result);
    frame(Some(&rt.lua), &state, 9.);
    assert_eq!(captured.borrow().len(), 3);
    rt.lua.load("local s=X2Rank:GetSnapshot();assert(s[1].name=='Expansion fixture' and s[2].name==string.char(255));X2Rank:RequestRankSnapshots()").exec()?;
    assert!(state.borrow().ranking.live.ready);
    assert_eq!(commands(&state), [ranking_command(Some(1))]);
    snapshot(Some(&rt), &state, 1, 200, vec![]);
    frame(Some(&rt.lua), &state, 10.);
    assert_eq!(captured.borrow().last().unwrap().0, "RANK_SNAPSHOTS");
    rt.lua.load("X2Rank:RequestPlayerRecords(0)").exec()?;
    dispatch(Some(&rt.lua), &state, &Event::Disconnected { message: "fixture".into() });
    assert!(commands(&state).is_empty());
    Ok(())
}

#[test]
fn original_calendar_clamps_maintenance_inclusive_bounds_and_non_overnight_hours() -> Result<(), Box<dyn std::error::Error>> {
    let lua = Lua::new();
    lua.load("localDay=1;localHour=0;os.date=function() return {wday=localDay,hour=localHour} end;calls={};os.time=function(t) calls[#calls+1]=t;return t.year==2014 and 1000 or 2000 end").exec()?;
    assert_eq!(query::calendar(&lua, [0, 1, 1, 0, 0]), Some(0));
    assert_eq!(query::calendar(&lua, [1800, -1, 99, 99, -1]), Some(2000));
    lua.load(
        "local t=calls[1];assert(t.year==1900 and t.month==1 and t.day==31 and t.hour==23 and t.min==0 and t.sec==0 and t.isdst==false)",
    )
    .exec()?;
    let db = rusqlite::Connection::open_with_flags(aa_anim::db::DEFAULT_DB, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut tables = Tables::load(&db)?;
    tables.rows[0].start = [2014, 1, 1, 0, 0];
    tables.rows[0].end = [2015, 1, 1, 0, 0];
    tables.rows[0].weekday = 8;
    tables.rows[0].start_hour = 0;
    tables.rows[0].end_hour = 0;
    let mut p = Projection {
        selected_rank: 1,
        ..Default::default()
    };
    for (ack, expected) in [(999, false), (1000, true), (2000, true), (2001, false)] {
        p.received.ack_time_raw = Some(ack);
        assert_eq!(query::activated(&lua, &p, &tables), expected, "ack {ack}");
    }
    p.received.ack_time_raw = Some(1500);
    for (hour, expected) in [(4, true), (5, false), (6, false), (7, true)] {
        lua.globals().set("localDay", 2)?;
        lua.globals().set("localHour", hour)?;
        assert_eq!(query::activated(&lua, &p, &tables), expected, "Monday {hour}");
    }
    tables.rows[0].weekday = 7;
    lua.globals().set("localHour", 20)?;
    for (day, expected) in [(6, false), (7, true), (1, false)] {
        lua.globals().set("localDay", day)?;
        assert_eq!(query::activated(&lua, &p, &tables), expected, "weekday {day}");
    }
    tables.rows[0].weekday = 8;
    tables.rows[0].start_hour = 20;
    tables.rows[0].end_hour = 22;
    for (hour, expected) in [(19, false), (20, true), (21, true), (22, false)] {
        lua.globals().set("localHour", hour)?;
        assert_eq!(query::activated(&lua, &p, &tables), expected, "hour {hour}");
    }
    tables.rows[0].start_hour = 8;
    tables.rows[0].end_hour = 4;
    for hour in [0, 3, 8, 23] {
        lua.globals().set("localHour", hour)?;
        assert!(!query::activated(&lua, &p, &tables));
    }
    p.selected_rank = 999;
    assert!(!query::activated(&lua, &p, &tables));
    Ok(())
}

#[test]
fn deferred_callbacks_preserve_pre_ack_errors_and_each_native_selected_board() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    let captured = Rc::new(RefCell::new(Vec::new()));
    let output = captured.clone();
    let s = state.clone();
    rt.lua.globals().set(
        "recordRankingState",
        rt.lua.create_function(move |_, event: String| {
            let data = s.borrow();
            let p = data.ranking.current();
            output.borrow_mut().push((
                event,
                p.received.ack_time_raw,
                p.default_tab,
                p.selected_tab,
                p.received.by_id.get(&p.selected_rank).map(|board| board.personal.v1),
            ));
            Ok(())
        })?,
    )?;
    rt.lua.load("local w=UIParent:CreateWidget('emptywidget','rankSnapshots','UIParent');w:RegisterEvent('SYSMSG');w:RegisterEvent('RANK_PLAYER_RECORDS');w:SetHandler('OnEvent',function(self,e) recordRankingState(e) end)").exec()?;
    state.borrow_mut().ranking.live.received.ack_time_raw = Some(11);
    records(None, &state, 22, vec![]);
    records(None, &state, 33, vec![personal(2, 11)]);
    records(None, &state, 44, vec![personal(1, 3), personal(2, 22)]);
    assert!(captured.borrow().is_empty());
    let pending = std::mem::take(&mut state.borrow_mut().ranking.pending);
    for effect in pending {
        apply_ui(&rt.lua, &state, effect)?;
    }
    assert_eq!(
        *captured.borrow(),
        [
            ("SYSMSG".into(), Some(11), 0, 0, None),
            ("RANK_PLAYER_RECORDS".into(), Some(22), 0, 0, None),
            ("RANK_PLAYER_RECORDS".into(), Some(33), 2, 2, Some(11)),
            ("RANK_PLAYER_RECORDS".into(), Some(44), 1, 2, Some(22)),
        ]
    );
    assert!(state.borrow().ranking.callback.is_none());
    rt.lua.load("X2Rank:RequestPlayerRecords(0)").exec()?;
    assert_eq!(commands(&state), [ranking_command(None)]);
    records(Some(&rt), &state, 55, vec![]);
    assert_eq!(state.borrow().ranking.live.selected_tab, 1);
    assert_eq!(state.borrow().ranking.live.selected_rank, 1);
    Ok(())
}

#[test]
fn original_ranking_tabs_refresh_and_reward_button_use_received_data_and_typed_requests() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::breath::fixture::runtime(1920., 1080.)?;
    state.borrow_mut().character_id = 47;
    let mut selected = aa_net::model::CharacterState::default();
    selected.character.id = 47;
    dispatch(Some(&rt.lua), &state, &Event::CharacterSelected(Box::new(selected)));
    dispatch(
        Some(&rt.lua),
        &state,
        &Event::PlayerSpawn(Box::new(super::super::expansion::fixture::spawn())),
    );
    rt.lua
        .load("rankFixture=ADDON:GetContent(UIC_RANK);ADDON:ShowContent(UIC_BAG,false);ADDON:ShowContent(UIC_RANK,true);assert(rankFixture:IsVisible())")
        .exec()?;
    assert_eq!(commands(&state), [ranking_command(None)]);
    records(
        Some(&rt),
        &state,
        1_700_000_000,
        vec![personal(1, 123), personal(2, 234), personal(3, 345)],
    );
    assert_eq!(commands(&state), [ranking_command(Some(1))]);
    name_reply(&state, 42, b"Received rival");
    snapshot(Some(&rt), &state, 1, 1_700_000_000, vec![entry(47, 1, 444), entry(42, 2, 333)]);
    let seconds = state.borrow().start.elapsed().as_secs_f32();
    frame(Some(&rt.lua), &state, seconds + 4.);
    aa_ui::lua::flush(&rt.lua);
    rt.lua.load("assert(not rankFixture.modalLoadingWindow:IsVisible());assert(rankFixture.content.upperFrame.myCurRecord:GetText():find('123',1,true))").exec()?;
    super::super::expansion::fixture::click(&rt, "rankFixture.content.upperFrame.rewardButton")?;
    rt.lua.load("assert(rankFixture.content.rewardWindow:IsVisible())").exec()?;
    aa_ui::lua::flush(&rt.lua);
    // The original popup opens with a centred scale/alpha animation; the physical
    // click helper samples its final layout, so advance the actual animation first.
    aa_ui::lua::tick(&rt.lua, 600.);
    super::super::expansion::fixture::click(&rt, "rankFixture.content.rewardWindow.okButton")?;
    super::super::expansion::fixture::click(&rt, "rankFixture.tab.unselectedButton[2]")?;
    assert_eq!(commands(&state), [ranking_command(None)]);
    records(
        Some(&rt),
        &state,
        1_700_000_001,
        vec![personal(1, 124), personal(2, 235), personal(3, 346)],
    );
    assert_eq!(commands(&state), [ranking_command(Some(2))]);
    snapshot(Some(&rt), &state, 2, 1_700_000_001, vec![entry(42, 1, 888)]);
    frame(Some(&rt.lua), &state, seconds + 5.);
    aa_ui::lua::flush(&rt.lua);
    rt.lua
        .load("assert(rankFixture.tabSelected==2 and not rankFixture.modalLoadingWindow:IsVisible())")
        .exec()?;
    super::super::expansion::fixture::click(&rt, "rankFixture.content.upperFrame.refreshButton")?;
    assert_eq!(commands(&state), [ranking_command(None)]);
    records(Some(&rt), &state, 1_700_000_002, vec![personal(2, 999)]);
    assert!(commands(&state).is_empty(), "original refresh updates only the personal record");
    rt.lua
        .load("assert(rankFixture.content.upperFrame.myCurRecord:GetText():find('999',1,true))")
        .exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn actual_hud_schedule_replays_loading_callbacks_and_writes_captured_board_and_name_requests() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua.load("local w=UIParent:CreateWidget('emptywidget','rankSchedule','UIParent');w:RegisterEvent('RANK_PLAYER_RECORDS');w:SetHandler('OnEvent',function() X2Rank:RequestRankSnapshots() end)").exec()?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(super::super::HudBuilt { up: false, fresh: false })
        .add_message::<NetEvent>()
        .add_message::<NetCommand>();
    plugin(&mut app);
    app.world_mut().write_message(NetEvent(Event::Ranking(RankingEvent::Personal {
        type_id: 0,
        error: 0,
        ack_time_raw: 100,
        count: 1,
        rows: vec![personal(1, 7)],
    })));
    app.update();
    assert!(app.world_mut().resource_mut::<Messages<NetCommand>>().drain().next().is_none());
    app.world_mut().resource_mut::<super::super::HudBuilt>().up = true;
    app.update();
    let commands = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|command| command.0)
        .collect::<Vec<_>>();
    assert!(matches!(
        commands.as_slice(),
        [aa_net::Command::Ranking {
            character_id: 47,
            command: RankingCommand::RequestSnapshots { rank_id: 1 }
        }]
    ));
    app.world_mut().write_message(NetEvent(Event::Ranking(RankingEvent::Snapshot {
        rank_id: 1,
        error: 0,
        board: RankSnapshot {
            board_id: 0,
            updated_raw: 100,
            reset_raw: 0,
            entry_count: 1,
            entries: vec![entry(42, 1, 4)],
            scope_count: 0,
            scopes: vec![],
        },
    })));
    app.update();
    let commands = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|command| command.0)
        .collect::<Vec<_>>();
    assert!(matches!(
        commands.as_slice(),
        [aa_net::Command::QueryCharacterName {
            character_id: 47,
            requested_id: 42
        }]
    ));
    assert!(!state.borrow().ranking.live.ready);
    app.world_mut().write_message(NetEvent(Event::CharacterName(aa_net::CharacterName {
        character_id: 42,
        name: vec![],
    })));
    app.update();
    assert!(state.borrow().ranking.live.ready);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Rank:RequestPlayerRecords(1)")
        .exec()?;
    app.world_mut()
        .write_message(NetEvent(Event::Disconnected { message: "fixture".into() }));
    app.update();
    assert!(app.world_mut().resource_mut::<Messages<NetCommand>>().drain().next().is_none());
    Ok(())
}
