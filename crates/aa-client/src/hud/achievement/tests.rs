use super::*;
use aa_net::{AchievementEvent as Receipt, AchievementProgress as Progress};
use std::{cell::RefCell, rc::Rc};

fn setup() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", 1280., 720.)?;
    let data = super::super::api::HudData::default();
    data.game
        .0
        .set(super::super::data::GameData::load(aa_anim::db::DEFAULT_DB)?)
        .ok()
        .unwrap();
    let bridge = Rc::new(RefCell::new(data));
    super::super::api::install(&mut rt, &bridge)?;
    Ok((rt, bridge))
}

fn enable(bridge: &Shared) {
    bridge.borrow_mut().achievement.cache.native_enabled = Some(true);
}

#[test]
fn actual_catalog_matches_all_38_executed_native_category_lists_and_loaded_order() -> Result<(), Box<dyn std::error::Error>> {
    let db = rusqlite::Connection::open_with_flags(aa_anim::db::DEFAULT_DB, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tables = Tables::load(&db)?;
    let oracle: serde_json::Value = serde_json::from_str(include_str!("native_lists.json"))?;
    let expected = oracle["catalog_ids_in_loaded_order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_u64().unwrap() as u32)
        .collect::<Vec<_>>();
    assert_eq!(tables.rows.iter().map(|entry| entry.id).collect::<Vec<_>>(), expected);
    assert_eq!(tables.rows.len(), 1743);
    assert!(tables.rows.iter().all(|entry| entry.rule.completed_amount() > 0));
    let view = View::default();
    for case in oracle["cases"].as_array().unwrap() {
        let got = query::list(
            &tables,
            &view,
            case["category"].as_i64().unwrap() as i32,
            case["sub"].as_i64().unwrap() as i32,
            case["filter"].as_i64().unwrap() as i32,
        )?;
        let expected = case["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_u64().unwrap() as u32)
            .collect::<Vec<_>>();
        assert_eq!(got, expected, "case {case}");
    }
    Ok(())
}

#[test]
fn executed_native_priority_unsigned_date_and_f32_ratio_comparisons() -> Result<(), Box<dyn std::error::Error>> {
    let oracle: serde_json::Value = serde_json::from_str(include_str!("native_comparisons.json"))?;
    for case in oracle["cases"].as_array().unwrap() {
        let db = rusqlite::Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE localized_texts(idx,en_us,tbl_name,tbl_column_name);CREATE TABLE icons(id,filename);CREATE TABLE char_records(id,kind_id,value1);CREATE TABLE achievement_objectives(achievement_id,record_id);CREATE TABLE achievements(id,name,summary,description,category_id,sub_category_id,parent_achievement_id,is_active,is_hidden,priority,complete_num,item_id,icon_id);")?;
        let mut view = View::default();
        for row in case["rows"].as_array().unwrap() {
            let n = |key: &str, fallback| row[key].as_i64().unwrap_or(fallback);
            let id = n("id", 0) as u32;
            db.execute(
                "INSERT INTO achievements VALUES(?1,'','','',1,1,?2,1,0,?3,?4,0,0)",
                rusqlite::params![id, n("parent", 0), n("priority", 0), n("complete_num", 10)],
            )?;
            for _ in 0..n("objective_count", 0) {
                db.execute("INSERT INTO achievement_objectives VALUES(?1,0)", [id])?;
            }
            view.cache.entries.insert(
                id,
                Progress {
                    id,
                    amount: n("amount", 0) as i32,
                    completed_at: n("timestamp", 0),
                },
            );
        }
        let tables = Tables::load(&db)?;
        let a = case["left"].as_u64().unwrap() as u32;
        let b = case["right"].as_u64().unwrap() as u32;
        let expected = case["native_less"].as_bool().unwrap();
        let less = match case["kind"].as_str().unwrap() {
            "priority" => query::category_order(&tables, a, b).is_lt(),
            kind => {
                let filter = if kind == "completed" { 2 } else { 4 };
                if filter == 4 && tables.rows.iter().any(|entry| entry.rule.completed_amount() == 0) {
                    assert!(
                        query::list(&tables, &view, 1, 1, 4).is_err(),
                        "zero-denominator native comparator is explicitly unsupported"
                    );
                    continue;
                }
                let ids = query::list(&tables, &view, 1, 1, filter)?;
                ids.iter().position(|&id| id == a).unwrap() < ids.iter().position(|&id| id == b).unwrap()
            }
        };
        assert_eq!(less, expected, "{}", case["label"]);
    }
    Ok(())
}

#[test]
fn info_typed_arguments_native_float_fields_objectives_and_invalid_dates() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    rt.lua
        .load(
            r#"
        assert(not X2Player:GetFeatureSet().achievement)
        assert(X2Achievement:GetMainCategoryCount()==5)
        for i,n in ipairs({4,4,5,3,3}) do assert(X2Achievement:GetSubCategoryCount(i)==n) end
        assert(X2Achievement:GetSubCategoryCount(6)==0)
        assert(select('#',X2Achievement:GetAchievementInfo('1'))==0)
        assert(select('#',X2Achievement:GetAchievementList(1,1,'1'))==0)
        local n=0;local v=X2Achievement:GetAchievementInfo(999999)
        for _ in pairs(v) do n=n+1 end
        assert(n==3 and v.current==0 and v.complete==false and v.tracing==false)
        assert(X2Achievement:GetAchievementInfo(0).tracing)
        assert(select('#',X2Achievement:GetTracingAchievementCount())==2)
        local n,m=X2Achievement:GetTracingAchievementCount();assert(n==0 and m==6)
    "#,
        )
        .exec()?;
    enable(&bridge);
    rt.lua.load("assert(X2Player:GetFeatureSet().achievement)").exec()?;
    let ids = bridge
        .borrow()
        .game
        .get()
        .unwrap()
        .achievements
        .rows
        .iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    for id in ids {
        let data = bridge.borrow();
        let entry = data.game.get().unwrap().achievements.get(id).unwrap();
        let table = info(&rt.lua, &data, id)?;
        assert_eq!(table.get::<f32>("type")?, id as f32);
        assert_eq!(table.get::<f32>("completeNum")?, entry.rule.completed_amount() as f32);
        assert_eq!(table.get::<f32>("reward")?, entry.reward as f32);
        let objectives = table.get::<Option<Table>>("objective")?;
        assert_eq!(objectives.is_none(), entry.objectives.is_empty());
        if let Some(objectives) = objectives {
            assert_eq!(
                objectives.sequence_values::<f32>().collect::<mlua::Result<Vec<_>>>()?,
                entry.objectives.iter().map(|&id| id as f32).collect::<Vec<_>>()
            );
        }
    }
    for timestamp in [-1, i64::MIN, 32_535_244_800, i64::MAX] {
        dispatch(
            None,
            &bridge,
            &Event::Achievement(Receipt::List {
                raw_count: 1,
                rows: vec![Progress {
                    id: 999_999,
                    amount: -7,
                    completed_at: timestamp,
                }],
            }),
        );
        bridge
            .borrow_mut()
            .achievement
            .cache
            .entries
            .get_mut(&999_999)
            .unwrap()
            .completed_at = timestamp;
        let table = info(&rt.lua, &bridge.borrow(), 999_999)?;
        assert_eq!(table.get::<i32>("current")?, -7);
        assert!(table.get::<bool>("complete")?);
        let date: Table = table.get("completeDate")?;
        assert_eq!(date.clone().pairs::<String, Value>().count(), 6);
        assert_eq!(
            (date.get::<i32>("year")?, date.get::<i32>("month")?, date.get::<i32>("day")?),
            (1899, 0, -1)
        );
    }
    let valid = date(&rt.lua, 1_792_276_001)?;
    let os: Table = rt.lua.globals().get("os")?;
    let local: Table = os.call_function("date", ("*t", 1_792_276_001i64))?;
    for (a, b) in [
        ("year", "year"),
        ("month", "month"),
        ("day", "day"),
        ("hour", "hour"),
        ("minute", "min"),
        ("second", "sec"),
    ] {
        assert_eq!(valid.get::<i32>(a)?, local.get::<i32>(b)?);
    }
    Ok(())
}

#[test]
fn received_completions_preserve_callback_order_duplicate_recent_and_trace_holes() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    enable(&bridge);
    rt.lua
        .load(
            r#"
        events={}
        UIParent:SetEventHandler('COMPLETE_ACHIEVEMENT',function(id)
            assert(X2Achievement:GetAchievementInfo(id).complete)
            assert(X2Achievement:GetAchievementInfo(id).isNew)
            table.insert(events,{'done',X2Achievement:IsTracingAchievement(id)})
        end)
        UIParent:SetEventHandler('ACHIEVEMENT_UPDATE',function() table.insert(events,{'update',X2Achievement:IsTracingAchievement(1)}) end)
        assert(X2Achievement:AddTracingAchievement(1))
        assert(X2Achievement:AddTracingAchievement(2))
        assert(not X2Achievement:AddTracingAchievement(0))
        assert(not X2Achievement:AddTracingAchievement(1))
        events={}
    "#,
        )
        .exec()?;
    let receive = |receipt| dispatch(Some(&rt), &bridge, &Event::Achievement(receipt));
    receive(Receipt::Completed {
        id: 1,
        completed_at: 1_792_276_001,
    });
    rt.lua
        .load(
            r#"
        assert(#events==3 and events[1][1]=='done' and events[1][2])
        assert(events[2][1]=='update' and not events[2][2] and events[3][1]=='update')
        assert(X2Achievement:GetNewCompleteAchievementCount()==1)
        assert(X2Achievement:GetAchievementInfo(1).current==X2Achievement:GetAchievementInfo(1).completeNum)
    "#,
        )
        .exec()?;
    assert_eq!(bridge.borrow().achievement.tracing, [0, 2, 0, 0, 0, 0]);
    receive(Receipt::Completed {
        id: 1,
        completed_at: 1_792_276_001,
    });
    rt.lua
        .load(
            r#"
        assert(#events==5 and events[4][1]=='done' and not events[4][2])
        assert(X2Achievement:GetNewCompleteAchievementCount()==2)
        X2Achievement:RemoveNewCompleteAchievement(1)
        assert(X2Achievement:GetNewCompleteAchievementCount()==1 and X2Achievement:GetAchievementInfo(1).isNew)
        X2Achievement:RemoveNewCompleteAchievement(1)
        assert(X2Achievement:GetNewCompleteAchievementCount()==0 and not X2Achievement:GetAchievementInfo(1).isNew)
        local n=#events;X2Achievement:RemoveNewCompleteAchievement(1);assert(#events==n)
        X2Achievement:RemoveAllNewCompleteAchievement();assert(#events==n+1)
        assert(X2Achievement:AddTracingAchievement(3))
        local list=X2Achievement:GetAchievementList(0,0,AF_TRACING);assert(list[1]==3 and list[2]==2)
        local n=#events;X2Achievement:RemoveTracingAchievement(0);assert(#events==n+1)
    "#,
        )
        .exec()?;
    assert_eq!(bridge.borrow().achievement.tracing, [3, 2, 0, 0, 0, 0]);
    let before = bridge.borrow().achievement.recent.len();
    bridge.borrow_mut().achievement.cache.native_enabled = Some(false);
    receive(Receipt::Completed { id: 2, completed_at: 123 });
    assert_eq!(bridge.borrow().achievement.recent.len(), before);
    assert!(!bridge.borrow().achievement.cache.get(2).is_some_and(|row| row.completed_at != 0));
    Ok(())
}

#[test]
fn actual_type5_push_hydrates_tracing_and_original_leave_saves_before_leave_packet() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    let spawn = super::super::expansion::fixture::spawn();
    let owner = spawn.character().id;
    dispatch(Some(&rt), &bridge, &Event::CharacterSelected(Box::new(spawn.state.clone())));
    bridge.borrow_mut().character_id = owner;
    let payload = "other saved\r\ntracing_achievement ( 2, 0, 0, 0, 0, 0 )\r\nversion 0\r\n";
    let receipt = aa_net::UiDataResponse {
        character_id: owner,
        data_type: 5,
        payload: payload.into(),
        raw_length: payload.len() as i32 + 1,
    };
    dispatch(Some(&rt), &bridge, &Event::UiData(aa_net::UiDataEvent::Response(receipt.clone())));
    rt.lua.load("assert(X2Achievement:IsTracingAchievement(2));assert(X2Achievement:AddTracingAchievement(3));local n,m=X2Achievement:GetTracingAchievementCount();assert(n==2 and m==6)").exec()?;
    let mut stale = receipt;
    stale.character_id = owner + 1;
    stale.payload.clear();
    stale.raw_length = 1;
    dispatch(Some(&rt), &bridge, &Event::UiData(aa_net::UiDataEvent::Response(stale)));
    assert_eq!(bridge.borrow().achievement.tracing, [2, 3, 0, 0, 0, 0]);
    assert!(save_for_leave(&mut bridge.borrow_mut(), 99).is_none());
    let mut app = App::new();
    app.init_resource::<NextState<crate::Screen>>()
        .init_resource::<aa_net::state::ActionBar>()
        .init_resource::<aa_net::state::Inventory>()
        .init_resource::<aa_net::state::Target>()
        .init_resource::<aa_net::state::Skills>()
        .init_resource::<aa_net::state::Buffs>()
        .add_message::<aa_net::NetCommand>()
        .add_message::<super::super::alerts::UseFailed>()
        .insert_non_send(HudBridge(bridge.clone()))
        .add_systems(Update, super::super::api::handle_requests);
    bridge.borrow_mut().requests.push(super::super::api::Request::Leave { how: 1 });
    app.update();
    let commands = app
        .world_mut()
        .resource_mut::<Messages<aa_net::NetCommand>>()
        .drain()
        .map(|event| event.0)
        .collect::<Vec<_>>();
    let [
        aa_net::Command::UiData(aa_net::UiDataCommand::Save {
            character_id,
            data_type,
            payload,
        }),
        aa_net::Command::LeaveWorld(aa_net::LeaveTarget::CharacterSelect),
    ] = commands.as_slice()
    else {
        panic!("unexpected leave command sequence: {commands:?}")
    };
    assert_eq!((*character_id, *data_type), (owner, 5));
    assert_eq!(payload, "other saved\r\ntracing_achievement ( 2, 3, 0, 0, 0, 0 )\r\nversion 1\r\n");
    dispatch(Some(&rt), &bridge, &Event::LeftWorld { target: 1 });
    assert!(save_for_leave(&mut bridge.borrow_mut(), 1).is_none());
    assert_eq!(bridge.borrow().achievement.tracing, [0; 6]);
    Ok(())
}

#[test]
#[ignore = "loads original world-stage scripts and exercises achievement tracing and detail controls"]
fn original_list_tracing_completion_detail_and_back_controls() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = fixture::runtime(1920., 1080.)?;
    let row = "ACHIEVEMENT_FIXTURE.contentWnd.listWnd.bodySectionWnd.frame.listCtrl.items[2].subItems[1].achievementWnd";
    // The original Show starts a native scale animation; physical coordinates settle afterward.
    aa_ui::lua::tick(&rt.lua, 600.);
    rt.lua
        .load(format!("assert({row}.achievementType==2 and {row}.tracingCheck:GetChecked())"))
        .exec()?;
    super::super::expansion::fixture::click(&rt, &format!("{row}.tracingCheck"))?;
    assert!(!bridge.borrow().achievement.tracing.contains(&2));
    super::super::expansion::fixture::click(&rt, &format!("{row}.tracingCheck"))?;
    assert!(bridge.borrow().achievement.tracing.contains(&2));
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Achievement(Receipt::Completed {
            id: 2,
            completed_at: 1_792_276_002,
        }),
    );
    assert!(!bridge.borrow().achievement.tracing.contains(&2));
    rt.lua
        .load(format!(
            "assert(not {row}.tracingCheck:IsVisible() and {row}.completeDate:IsVisible())"
        ))
        .exec()?;
    rt.lua
        .load("ACHIEVEMENT_FIXTURE.contentWnd:ShowAchievementDetail(2,{viewType=AVT_LIST,list=X2Achievement:GetAchievementList(1,1,1)})")
        .exec()?;
    rt.lua.load("assert(ACHIEVEMENT_FIXTURE.contentWnd.detailWnd:IsVisible());assert(ACHIEVEMENT_FIXTURE.contentWnd.detailWnd.achievementType==2);assert(not X2Achievement:GetAchievementInfo(2).isNew)").exec()?;
    super::super::expansion::fixture::click(&rt, "ACHIEVEMENT_FIXTURE.contentWnd.detailWnd.upperSectionWnd.prevPageBtn")?;
    rt.lua.load("assert(ACHIEVEMENT_FIXTURE.contentWnd.listWnd:IsVisible() and not ACHIEVEMENT_FIXTURE.contentWnd.detailWnd:IsVisible());ACHIEVEMENT_FIXTURE:ShowTracingAchivementList();assert(#X2Achievement:GetAchievementList(0,0,AF_TRACING)==1)").exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    drop(rt);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
