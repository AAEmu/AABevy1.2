use super::*;
use aa_net::battlefield::BattlefieldQueue;

#[test]
#[ignore = "loads original world-stage scripts and exercises physical battlefield entrance controls"]
fn original_recruiter_apply_queue_and_synchronous_cancel_controls() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::fixture::runtime(1920., 1080.)?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge.clone()))
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, update);
    app.update();
    crate::hud::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "battlefield.entranceWnd.contentFrame[1].entranceRandomBtn")?;
    app.update();
    let drain = |app: &mut App| app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|v| v.0).collect::<Vec<_>>();
    let commands = drain(&mut app);
    assert!(matches!(
        commands.as_slice(),
        [aa_net::Command::Battlefield(BattlefieldCommand::Apply { battlefield_id: 1, corps: 2 })]
    ));
    assert!(bridge.borrow().battlefield.received.applied.is_none());
    let command = BattlefieldCommand::Apply { battlefield_id: 1, corps: 2 };
    for event in [
        BattlefieldEvent::Requested(command),
        BattlefieldEvent::Applied {
            battlefield_id: 1,
            corps: 2,
            error: 0,
        },
        BattlefieldEvent::QueueState(BattlefieldQueue {
            battlefield_id: 1,
            corps_size: 1,
            counts: [1, 1, 0],
        }),
    ] {
        app.world_mut().write_message(NetEvent(Event::Battlefield(event)));
    }
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load(
            r#"
        local n,m,c=X2BattleField:GetJoinedInfos();assert(n==2 and m==1 and c==2)
        assert(battlefield.entranceAlarm:IsVisible())
        assert(battlefield.entranceWnd.contentFrame[1].cancelEntranceBtn:IsVisible())
    "#,
        )
        .exec()?;
    crate::hud::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "battlefield.entranceWnd.contentFrame[1].cancelEntranceBtn")?;
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load(
            r#"
        local n,m,c=X2BattleField:GetJoinedInfos();assert(n==0 and m==0 and c==-1)
        assert(not battlefield.entranceWnd:IsVisible())
        assert(battlefield.entranceAlarm==nil)
    "#,
        )
        .exec()?;
    app.update();
    assert!(matches!(drain(&mut app).as_slice(), [aa_net::Command::Battlefield(BattlefieldCommand::Cancel)]));
    assert!(
        bridge.borrow().battlefield.received.applied.is_some(),
        "only local UI resets before the cancel receipt"
    );
    assert!(app.world().non_send::<UiRuntime>().errors.is_empty());
    drop(app);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub(super) fn setup() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", 1280., 720.)?;
    let mut data = HudData::default();
    data.game.0.set(super::super::data::GameData::load(aa_anim::db::DEFAULT_DB)?).ok().unwrap();
    data.character_id = 47;
    data.battlefield.owner = 47;
    data.battlefield.level = Some(45);
    data.battlefield.buffs = Some(vec![]);
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(data));
    super::super::api::install(&mut rt, &bridge)?;
    Ok((rt, bridge))
}

#[test]
fn original_entrance_getters_use_npc_mapping_descending_bands_and_zero_result_sentinels() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    rt.lua
        .load(
            r#"
        assert(X2BattleField:GetInteractedNpcType()==0)
        assert(select('#',X2BattleField:GetEntranceBasicInfo(0))==0)
        local info=X2BattleField:GetEntranceBasicInfo(3)
        assert(info.type==1 and type(info.name)=='string' and #info.name>0 and type(info.desc)=='string')
        assert(X2BattleField:GetEntranceMaximumPlayers(3)==1)
        local min,max=X2BattleField:GetEntranceLevelSection(3); assert(min==45 and max==55)
        local victory=X2BattleField:GetEntranceVictoryConditionInfo(3)
        assert(victory.victoryScore==0 and victory.victoryKillCount==0 and victory.victoryTarget==13353)
        assert(X2BattleField:GetEntranceFactionInfo(3,2)=='ANY_CORPS')
        assert(X2BattleField:GetEntranceFactionInfo(3,9)=='')
        assert(#X2BattleField:GetEntranceFactionInfo(3,0)>0)
    "#,
        )
        .exec()?;
    bridge.borrow_mut().battlefield.level = Some(40);
    rt.lua
        .load("local min,max=X2BattleField:GetEntranceLevelSection(3);assert(min==40 and max==44 and X2BattleField:GetEntranceMaximumPlayers(3)==2)")
        .exec()?;
    bridge.borrow_mut().battlefield.level = Some(34);
    rt.lua
        .load(
            r#"
        local min,max=X2BattleField:GetEntranceLevelSection(3);assert(min==0 and max==34)
        assert(select('#',X2BattleField:GetEntranceMaximumPlayers(3))==0)
        assert(select('#',X2BattleField:GetEntranceFactionInfo(3,2))==0)
        assert(select('#',X2BattleField:GetEntranceVictoryConditionInfo(3))==0)
    "#,
        )
        .exec()?;
    bridge.borrow_mut().battlefield.level = None;
    rt.lua
        .load("assert(select('#',X2BattleField:GetEntranceBasicInfo(3))==0 and select('#',X2BattleField:GetEntranceLevelSection(3))==0)")
        .exec()?;
    Ok(())
}

#[test]
fn local_requests_keep_received_counts_separate_and_callbacks_observe_each_queue_packet() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    rt.lua
        .load(
            r#"
        notices={}
        UIParent:SetEventHandler('UPDATE_INSTANT_GAME_STATE',function(...)
            assert(select('#',...)==0)
            local count,size,corps=X2BattleField:GetJoinedInfos()
            notices[#notices+1]={count,size,corps}
        end)
        UIParent:SetEventHandler('INSTANT_GAME_JOIN_CANCEL',function(...)
            assert(select('#',...)==0)
            local count,size,corps=X2BattleField:GetJoinedInfos()
            assert(count==0 and size==0 and corps==-1 and X2BattleField:GetInteractedNpcType()==0)
            notices[#notices+1]='cancel'
        end)
        assert(select('#',X2BattleField:RequestInstanceGame(3,19))==0)
        local count,size,corps=X2BattleField:GetJoinedInfos();assert(count==0 and size==0 and corps==2)
        assert(X2BattleField:GetInteractedNpcType()==3 and not X2BattleField:IsInInstantGame())
        X2BattleField:RequestInstanceGame(3,0)
    "#,
        )
        .exec()?;
    let command = BattlefieldCommand::Apply { battlefield_id: 1, corps: 2 };
    assert_eq!(flush(&mut bridge.borrow_mut()), [command]);
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Requested(command)));
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Applied {
            battlefield_id: 1,
            corps: 2,
            error: 0,
        }),
    );
    let packet = Event::Battlefield(BattlefieldEvent::QueueState(BattlefieldQueue {
        battlefield_id: 1,
        corps_size: 5,
        counts: [9, 1, 11],
    }));
    dispatch(&rt, &bridge, &packet);
    dispatch(&rt, &bridge, &packet);
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::QueueState(BattlefieldQueue {
            battlefield_id: 7,
            corps_size: 9,
            counts: [9, 9, 9],
        })),
    );
    rt.lua
        .load(
            r#"
        assert(#notices==2 and notices[1][1]==10 and notices[1][2]==5 and notices[1][3]==2)
        assert(select('#',X2BattleField:CancelInstanceGame())==0 and notices[3]=='cancel')
        X2BattleField:RequestInstanceGame(3,0)
    "#,
        )
        .exec()?;
    assert_eq!(
        bridge.borrow().battlefield.received.joined_infos(),
        (10, 5, 2),
        "local cancellation does not fabricate received reset"
    );
    assert_eq!(flush(&mut bridge.borrow_mut()), [BattlefieldCommand::Cancel]);
    dispatch(&rt, &bridge, &packet);
    rt.lua.load("assert(#notices==3)").exec()?;
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Cancelled { raw: 0 }));
    rt.lua
        .load("local n,m,c=X2BattleField:GetJoinedInfos();assert(n==0 and m==0 and c==-1);X2BattleField:RequestInstanceGame(3,1)")
        .exec()?;
    assert_eq!(flush(&mut bridge.borrow_mut()), [BattlefieldCommand::Apply { battlefield_id: 1, corps: 1 }]);
    Ok(())
}

#[test]
fn blockers_unknown_history_and_stale_character_prevent_sends_without_inventing_level_guards() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    bridge.borrow_mut().battlefield.buffs = None;
    rt.lua.load("X2BattleField:RequestInstanceGame(3,0)").exec()?;
    assert!(flush(&mut bridge.borrow_mut()).is_empty());
    bridge.borrow_mut().battlefield.buffs = Some(vec![631]);
    rt.lua.load("cancelled=false;UIParent:SetEventHandler('INSTANT_GAME_JOIN_CANCEL',function()cancelled=true end);X2BattleField:RequestInstanceGame(3,0);assert(cancelled)").exec()?;
    assert_eq!(flush(&mut bridge.borrow_mut()), [BattlefieldCommand::Cancel]);
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Cancelled { raw: 0 }));
    {
        let mut data = bridge.borrow_mut();
        data.battlefield.buffs = Some(vec![]);
        data.battlefield.level = Some(1);
    }
    rt.lua.load("X2BattleField:RequestInstanceGame(3,0)").exec()?;
    assert_eq!(
        flush(&mut bridge.borrow_mut()),
        [BattlefieldCommand::Apply { battlefield_id: 1, corps: 0 }],
        "native preflight has no level check"
    );
    rt.lua.load("X2BattleField:CancelInstanceGame()").exec()?;
    bridge.borrow_mut().character_id = 48;
    assert!(flush(&mut bridge.borrow_mut()).is_empty());
    Ok(())
}

#[test]
fn application_errors_reset_only_matching_requests_and_transport_failure_keeps_local_busy() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    rt.lua.load("X2BattleField:RequestInstanceGame(3,1)").exec()?;
    let command = flush(&mut bridge.borrow_mut())[0];
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::RequestRejected {
            command,
            message: "fixture transport failure".into(),
        }),
    );
    rt.lua
        .load("local n,m,c=X2BattleField:GetJoinedInfos();assert(c==1);X2BattleField:RequestInstanceGame(3,0)")
        .exec()?;
    assert!(flush(&mut bridge.borrow_mut()).is_empty());
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Requested(command)));
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Applied {
            battlefield_id: 1,
            corps: 0,
            error: 106,
        }),
    );
    assert_eq!(bridge.borrow().battlefield.local_request, Some((1, 1)));
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Applied {
            battlefield_id: 1,
            corps: 1,
            error: 106,
        }),
    );
    rt.lua
        .load("local n,m,c=X2BattleField:GetJoinedInfos();assert(n==0 and m==0 and c==-1);assert(X2BattleField:GetInteractedNpcType()==0)")
        .exec()?;
    Ok(())
}
