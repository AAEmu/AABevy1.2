use super::*;
use aa_net::battlefield::{
    BattlefieldEnd, BattlefieldJoined, BattlefieldKill, BattlefieldPoint, BattlefieldSkillEnable, BattlefieldStart, BattlefieldTeamResult,
};

fn joined(rt: &UiRuntime, bridge: &Shared) {
    dispatch(rt, bridge, &Event::PlayerSpawn(Box::new(super::super::expansion::fixture::spawn())));
    let invitation = aa_net::battlefield::BattlefieldInvitation {
        zone_id: 501,
        instance_id: 9,
        ruleset_id: 27,
        corps: 1,
        qualified_id: 91,
    };
    let mut ids = [0; 24];
    ids[0] = 12399;
    for event in [
        BattlefieldEvent::Requested(BattlefieldCommand::Apply { battlefield_id: 1, corps: 2 }),
        BattlefieldEvent::Applied {
            battlefield_id: 1,
            corps: 2,
            error: 0,
        },
        BattlefieldEvent::Invited(invitation),
        BattlefieldEvent::Requested(BattlefieldCommand::RespondToInvitation { invitation, accept: true }),
        BattlefieldEvent::Joined(BattlefieldJoined {
            zone_id: 501,
            instance_id: 9,
            corps: 1,
            victory_score: 100,
            victory_kill_count: 50,
            victory_kill_corps_head: 0,
            phase_times_ms: [6000, 120000, 30000],
            factions: [159, 160],
            killstreak_skills: ids,
        }),
    ] {
        dispatch(rt, bridge, &Event::Battlefield(event));
    }
}

fn clock(rt: &UiRuntime, bridge: &Shared) {
    let now = std::time::Instant::now();
    dispatch(
        rt,
        bridge,
        &Event::ClockSample(aa_net::ClockSample {
            pong: aa_net::proxy::ProxyPong {
                tm: 0,
                when: 0,
                elapsed: 0,
                remote: 0,
                local: 0,
                world: 100000,
            },
            local_receive_ms: 0,
            received_at: now,
            local_origin: now,
        }),
    );
}

#[test]
fn progress_time_keys_wrapping_and_phase_callbacks_observe_native_state() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = super::tests::setup()?;
    joined(&rt, &bridge);
    rt.lua.load("assert(select('#',X2BattleField:GetProgressTimeInfo())==0)").exec()?;
    let time = progress::time_at(&rt.lua, &bridge.borrow(), u32::MAX)?.unwrap();
    assert_eq!(time.get::<String>("state")?, "JOINED");
    assert!(matches!(time.get::<Value>("startTime")?, Value::Nil));
    assert!(matches!(time.get::<Value>("endTime")?, Value::Nil));
    assert_eq!(time.get::<f64>("nowtime")?, f64::from(u32::MAX as f32));
    clock(&rt, &bridge);
    rt.lua
        .load(
            r#"
        phases={}
        UIParent:SetEventHandler('INSTANT_GAME_WAIT',function(...)
          assert(select('#',...)==0);phases[#phases+1]=X2BattleField:GetProgressTimeInfo().state
        end)
        UIParent:SetEventHandler('INSTANT_GAME_START',function(...)
          assert(select('#',...)==0);phases[#phases+1]=X2BattleField:GetProgressTimeInfo().state
        end)
        UIParent:SetEventHandler('INSTANT_GAME_END',function(...)
          assert(select('#',...)==0);phases[#phases+1]=X2BattleField:GetProgressTimeInfo().state
        end)
    "#,
        )
        .exec()?;
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Started(BattlefieldStart {
            zone_id: 501,
            instance_id: 9,
            now_ms: 1,
            start_ms: 160000,
        })),
    );
    let opening = bridge.borrow().battlefield.received.opening_at_ms.unwrap();
    let time = progress::time_at(&rt.lua, &bridge.borrow(), u32::MAX)?.unwrap();
    assert_eq!(time.get::<String>("state")?, "COUNTDOWN");
    assert_eq!(time.get::<f64>("startTime")?, f64::from(opening as f32));
    assert_eq!(time.get::<f64>("endTime")?, 160000.);
    assert_eq!(time.get::<f64>("nowtime")?, 5999.);
    notifications::advance_at(&rt, &bridge, 153999);
    assert_eq!(bridge.borrow().battlefield.received.phase, Some(aa_net::battlefield::BattlefieldPhase::Opening));
    notifications::advance_at(&rt, &bridge, 154000);
    notifications::advance_at(&rt, &bridge, 154001);
    let time = progress::time_at(&rt.lua, &bridge.borrow(), 160001)?.unwrap();
    assert_eq!(time.get::<String>("state")?, "STARTED");
    assert_eq!(time.get::<f64>("startTime")?, 160000.);
    assert_eq!(time.get::<f64>("endTime")?, 280000.);
    rt.lua.load("assert(#phases==2 and phases[1]=='COUNTDOWN' and phases[2]=='STARTED')").exec()?;
    rt.lua.load("X2BattleField:AskLeaveInstantGame()").exec()?;
    assert!(flush(&mut bridge.borrow_mut()).is_empty());
    let team = |state| BattlefieldTeamResult {
        state,
        faction_id: 0,
        score: 0,
        total_kills: 0,
        total_deaths: 0,
        members: vec![],
    };
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Ended(BattlefieldEnd {
            zone_id: 501,
            instance_id: 9,
            reason: 2,
            teams: [team(1), team(2)],
        })),
    );
    let ending = bridge.borrow().battlefield.received.ending_at_ms.unwrap();
    let time = progress::time_at(&rt.lua, &bridge.borrow(), 170001)?.unwrap();
    assert_eq!(time.get::<String>("state")?, "ENDED");
    assert_eq!(time.get::<f64>("startTime")?, f64::from(ending as f32));
    assert_eq!(time.get::<f64>("endTime")?, f64::from(ending.wrapping_add(30000) as f32));
    rt.lua
        .load("assert(#phases==3 and phases[3]=='ENDED');X2BattleField:AskLeaveInstantGame()")
        .exec()?;
    assert_eq!(flush(&mut bridge.borrow_mut()), [BattlefieldCommand::Leave]);
    assert!(bridge.borrow().battlefield.received.in_instant_game());
    Ok(())
}

#[test]
fn immediate_start_wait_callback_sees_countdown_before_playing() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = super::tests::setup()?;
    joined(&rt, &bridge);
    clock(&rt, &bridge);
    rt.lua
        .load(
            r#"
        states={}
        UIParent:SetEventHandler('INSTANT_GAME_WAIT',function()states[#states+1]=X2BattleField:GetProgressTimeInfo().state end)
        UIParent:SetEventHandler('INSTANT_GAME_START',function()states[#states+1]=X2BattleField:GetProgressTimeInfo().state end)
    "#,
        )
        .exec()?;
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Started(BattlefieldStart {
            zone_id: 501,
            instance_id: 9,
            now_ms: u32::MAX,
            start_ms: 99999,
        })),
    );
    rt.lua.load("assert(#states==2 and states[1]=='COUNTDOWN' and states[2]=='STARTED')").exec()?;
    Ok(())
}

#[test]
fn native_receive_callbacks_preserve_local_kill_notification_order_and_unguarded_notice() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = super::tests::setup()?;
    joined(&rt, &bridge);
    rt.lua
        .load(
            r#"
        pointEvents=0;killEvents=0;skillEvents=0;remainingEvents=0;order={}
        UIParent:SetEventHandler('INSTANT_GAME_ADD_POINT',function(corps,name,delta)
          local s=X2BattleField:GetProgressScoreInfo()
          assert(corps=='ANY_CORPS' and name=='Scorer' and delta==16777216 and s.score1==12)
          pointEvents=pointEvents+1
        end)
        UIParent:SetEventHandler('UPDATE_INSTANT_GAME_KILLSTREAK_COUNT',function(value)
          local s=X2BattleField:GetProgressScoreInfo()
          assert(value==0 and s.myKillStreakPoint==-7 and s.killScore1==0)
          order[#order+1]='local'
        end)
        UIParent:SetEventHandler('INSTANT_GAME_KILL',function(kill)
          assert(kill.killer=='Killer' and kill.victim=='Victim' and kill.killerKillstreak==-7 and kill.killerCorpsKill==9 and kill.victimCorpsDeath==100)
          local fields=0;for k,v in pairs(kill)do fields=fields+1 end;assert(fields==7)
          assert(X2BattleField:GetProgressScoreInfo().killScore1==9)
          order[#order+1]='kill';killEvents=killEvents+1
        end)
        UIParent:SetEventHandler('UPDATE_INSTANT_GAME_KILLSTREAK',function(...)assert(select('#',...)==0);skillEvents=skillEvents+1 end)
        UIParent:SetEventHandler('INSTANT_GAME_UNEARNED_WIN_REMAIN_TIME',function(seconds)assert(seconds==2);remainingEvents=remainingEvents+1 end)
    "#,
        )
        .exec()?;
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Started(BattlefieldStart {
            zone_id: 501,
            instance_id: 9,
            now_ms: 0,
            start_ms: 10000,
        })),
    );
    let point = BattlefieldPoint {
        zone_id: 501,
        instance_id: 9,
        corps: 2,
        points: 16777217,
        scores: [12, 34],
        character_name: "Scorer".into(),
    };
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Point(point.clone())));
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Point(point.clone())));
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Point(BattlefieldPoint { instance_id: 10, ..point })),
    );
    let kill = BattlefieldKill {
        zone_id: 501,
        instance_id: 9,
        killer_id: 88,
        victim_id: 47,
        killer_corps: 0,
        victim_corps: 1,
        killer_killstreak: -7,
        killer_name: "Killer".into(),
        victim_name: "Victim".into(),
        killer_corps_kills: 9,
        victim_corps_deaths: 100,
    };
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Kill(kill.clone())));
    rt.lua
        .load("assert(pointEvents==2 and killEvents==1 and #order==2 and order[1]=='local' and order[2]=='kill')")
        .exec()?;
    // Even an unmatched match updates the local streak and emits its local callback before rejecting main counts.
    bridge.borrow_mut().battlefield.received.kill_scores = [0; 2];
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Kill(BattlefieldKill { instance_id: 10, ..kill })),
    );
    assert_eq!(bridge.borrow().battlefield.received.kill_scores, [0; 2]);
    notifications::advance_at(&rt, &bridge, 4000);
    let enable = BattlefieldSkillEnable {
        zone_id: 501,
        instance_id: 9,
        killstreak: 0,
        skill_id: 12399,
        enabled: true,
    };
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::SkillEnable(enable)));
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::SkillEnable(enable)));
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::UnearnedWinRemaining { milliseconds: 1500 }));
    rt.lua
        .load("assert(skillEvents==1 and X2BattleField:GetMyKillstreakSkillsInfos()[1].enable and #order==3 and remainingEvents==1)")
        .exec()?;
    Ok(())
}

#[test]
fn result_members_keep_native_cap_flags_zero_based_own_index_and_integer_ratio() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = super::tests::setup()?;
    joined(&rt, &bridge);
    rt.lua
        .load("local t,i=X2BattleField:GetMembersDetailInfo();assert(#t==2 and #t[1]==0 and #t[2]==0 and i==0)")
        .exec()?;
    let team = |state| BattlefieldTeamResult {
        state,
        faction_id: 0,
        score: 0,
        total_kills: 0,
        total_deaths: 0,
        members: vec![],
    };
    let mut result = BattlefieldEnd {
        zone_id: 501,
        instance_id: 9,
        reason: 1,
        teams: [team(1), team(2)],
    };
    for index in 0..52 {
        result.teams[1].members.push(aa_net::battlefield::BattlefieldMemberResult {
            character_id: if index == 7 { 47 } else { 100 + index },
            name: format!("Member {index}"),
            score: 16777217,
            bonus: -5,
            bonus_set: 0xffff_fff5,
            killstreak: 65535,
            kills: 12,
            assists: 34,
            deaths: 56,
        });
    }
    bridge.borrow_mut().battlefield.received.result = Some(result);
    rt.lua
        .load(
            r#"
      local t,i=X2BattleField:GetMembersDetailInfo();assert(#t==2 and #t[1]==0 and #t[2]==50 and i==7)
      local m=t[2][8];local n=0;for k,v in pairs(m)do n=n+1 end;assert(n==12)
      assert(m.name=='Member 7' and m.memberScores==16777216 and m.memberBonus==-5 and m.memberKillstreaks==65535)
      assert(m.memberNPcKills==12 and m.memberNPcKillAssists==34 and m.memberNDeaths==56)
      assert(m.isWinner and not m.noDeath and m.topPcKill and not m.topNonPcKill and m.isLoser)
    "#,
        )
        .exec()?;
    for (ratio, factor) in [(99, 0), (199, 1), (200, 2), (-1, 1)] {
        bridge.borrow_mut().battlefield.received.my_corps_kill_death_ratio = ratio;
        rt.lua
            .load(format!(
                "local t=X2BattleField:GetMembersDetailInfo();assert(t[2][1].memberScores==16777216*{factor} and t[2][1].memberBonus==-5*{factor})"
            ))
            .exec()?;
    }
    Ok(())
}

#[test]
fn killstreak_slot_identity_tag_first_duplicate_enable_and_ordinary_darkness() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = super::tests::setup()?;
    joined(&rt, &bridge);
    {
        let mut data = bridge.borrow_mut();
        let view = &mut data.battlefield;
        let joined = view.received.joined.as_mut().unwrap();
        joined.killstreak_skills[1] = 12399;
        joined.killstreak_skills[2] = u32::MAX;
        view.received.skills_enabled[1] = true;
        data.snapshot.hp = 100;
        data.snapshot.mp = 100000;
        data.snapshot.level = 55;
        data.panels.target = None;
    }
    slots::refresh(&World::new(), &mut bridge.borrow_mut());
    rt.lua
        .load(
            r#"
      assert(AAUI.SlotContent(ISLOT_INSTANT_KILL_STREAK,0)==nil)
      assert(AAUI.SlotContent(ISLOT_INSTANT_KILL_STREAK,25)==nil)
      assert(AAUI.SlotContent(ISLOT_INSTANT_KILL_STREAK,3)==nil)
      local a=AAUI.SlotContent(ISLOT_INSTANT_KILL_STREAK,1)
      local b=AAUI.SlotContent(ISLOT_INSTANT_KILL_STREAK,2)
      assert(a.type=='skill' and a.id==12399 and #a.icon>0 and not a.canUse and not b.canUse)
    "#,
        )
        .exec()?;
    bridge.borrow_mut().battlefield.received.skills_enabled[0] = true;
    rt.lua
        .load("assert(AAUI.SlotContent(ISLOT_INSTANT_KILL_STREAK,1).canUse and AAUI.SlotContent(ISLOT_INSTANT_KILL_STREAK,2).canUse)")
        .exec()?;
    bridge.borrow_mut().snapshot.dead = true;
    slots::refresh(&World::new(), &mut bridge.borrow_mut());
    rt.lua.load("assert(not AAUI.SlotContent(ISLOT_INSTANT_KILL_STREAK,1).canUse)").exec()?;
    Ok(())
}

#[test]
#[ignore = "loads original world-stage battlefield countdown, actionbar, scores and result controls"]
fn original_match_phases_scoreboard_and_received_only_leave_controls() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::fixture::runtime(1920., 1080.)?;
    super::fixture::join(&rt, &bridge)?;
    rt.lua
        .load(
            r#"
        assert(X2BattleField:GetProgressTimeInfo().state=='JOINED')
        assert(battlefield.waitingAlarm and battlefield.waitingAlarm:IsVisible())
        assert(battlefield.actionbar and battlefield.actionbar:IsVisible())
        assert(battlefield.miniScoreboard==nil)
    "#,
        )
        .exec()?;
    super::fixture::play(&rt, &bridge);
    aa_ui::lua::tick(&rt.lua, 20.);
    rt.lua
        .load(
            r#"
        assert(X2BattleField:GetProgressTimeInfo().state=='STARTED')
        assert(battlefield.miniScoreboard and battlefield.miniScoreboard:IsVisible())
        assert(battlefield.actionbar.killstreakLabel:GetText()=='3')
        assert(battlefield.miniScoreboard.scoreTeam_2:GetText()=='12')
    "#,
        )
        .exec()?;
    super::fixture::end(&rt, &bridge);
    // Native alarm animation owns the ten-second reveal; advance its real update callbacks.
    for _ in 0..130 {
        aa_ui::lua::tick(&rt.lua, 100.);
    }
    rt.lua
        .load(
            r#"
        assert(X2BattleField:GetProgressTimeInfo().state=='ENDED')
        assert(battlefield.miniScoreboard==nil and battlefield.actionbar==nil)
        assert(battlefield.scoreboard and battlefield.scoreboard:IsVisible())
        local t,i=X2BattleField:GetMembersDetailInfo()
        assert(#t[1]==1 and #t[2]==1 and t[2][1].name=='Fixture player' and i==0)
    "#,
        )
        .exec()?;
    crate::hud::expansion::fixture::click(&rt, "battlefield.scoreboard.askLeave")?;
    assert_eq!(flush(&mut bridge.borrow_mut()), [BattlefieldCommand::Leave]);
    assert!(bridge.borrow().battlefield.received.in_instant_game());
    dispatch(
        &rt,
        &bridge,
        &Event::LoadInstance(aa_net::world_msg::InstanceLoad {
            raw_id: 1,
            zone_id: 2,
            pos: Default::default(),
            angles: [0.; 3],
        }),
    );
    rt.lua
        .load("assert(not X2BattleField:IsInInstantGame() and battlefield.scoreboard==nil)")
        .exec()?;
    assert!(rt.errors.is_empty());
    drop(rt);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn native_progress_slots_round_numbers_and_keep_global_scores_separate_from_received_team_totals() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = super::tests::setup()?;
    rt.lua
        .load(
            r#"
        local skills=X2BattleField:GetMyKillstreakSkillsInfos()
        assert(#skills==24 and skills.skill_count==0 and skills[24].skill_type==0 and skills[24].enable==false)
        assert(select('#',X2BattleField:GetProgressScoreInfo())==0)
        assert(select('#',X2BattleField:GetProgressVictoryConditionInfo())==0)
    "#,
        )
        .exec()?;
    dispatch(&rt, &bridge, &Event::PlayerSpawn(Box::new(super::super::expansion::fixture::spawn())));
    let command = BattlefieldCommand::Apply { battlefield_id: 1, corps: 2 };
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
    let invitation = aa_net::battlefield::BattlefieldInvitation {
        zone_id: 501,
        instance_id: 9,
        ruleset_id: 27,
        corps: 1,
        qualified_id: 91,
    };
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Invited(invitation)));
    let command = BattlefieldCommand::RespondToInvitation { invitation, accept: true };
    dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Requested(command)));
    let mut ids = [0; 24];
    ids[0] = 16_777_217;
    ids[2] = 42;
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Joined(BattlefieldJoined {
            zone_id: 501,
            instance_id: 9,
            corps: 1,
            victory_score: 16_777_217,
            victory_kill_count: 50,
            victory_kill_corps_head: 13353,
            phase_times_ms: [1000, 2000, 3000],
            factions: [159, 160],
            killstreak_skills: ids,
        })),
    );
    rt.lua
        .load(
            r#"
        local skills=X2BattleField:GetMyKillstreakSkillsInfos()
        assert(#skills==24 and skills.skill_count==2 and skills[1].skill_type==16777216 and skills[3].skill_type==42)
        local v=X2BattleField:GetProgressVictoryConditionInfo()
        assert(v.victoryScore==16777216 and v.victoryKillCount==50 and v.victoryTarget==13353)
    "#,
        )
        .exec()?;
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Started(BattlefieldStart {
            zone_id: 501,
            instance_id: 9,
            now_ms: 9999999,
            start_ms: 10000,
        })),
    );
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Point(BattlefieldPoint {
            zone_id: 501,
            instance_id: 9,
            corps: 0,
            points: 5,
            scores: [16_777_217, -16_777_217],
            character_name: "Scorer".into(),
        })),
    );
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Kill(BattlefieldKill {
            zone_id: 501,
            instance_id: 9,
            killer_id: 88,
            victim_id: 47,
            killer_corps: 0,
            victim_corps: 1,
            killer_killstreak: 7,
            killer_name: "Killer".into(),
            victim_name: "Victim".into(),
            killer_corps_kills: 9,
            victim_corps_deaths: 100,
        })),
    );
    rt.lua
        .load(
            r#"
        local s=X2BattleField:GetProgressScoreInfo()
        assert(s.score1==16777216 and s.score2==-16777216 and s.killScore1==9)
        assert(s.myKillStreakPoint==7 and s.myCorpsKill==0 and s.myCorpsDeath==0)
    "#,
        )
        .exec()?;
    assert!(bridge.borrow_mut().battlefield.received.advance_server_time(4000));
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::SkillEnable(BattlefieldSkillEnable {
            zone_id: 501,
            instance_id: 9,
            killstreak: 0,
            skill_id: 16_777_217,
            enabled: true,
        })),
    );
    rt.lua.load("assert(X2BattleField:GetMyKillstreakSkillsInfos()[1].enable)").exec()?;
    let team = |state, faction_id, total_kills, total_deaths| BattlefieldTeamResult {
        state,
        faction_id,
        score: 123,
        total_kills,
        total_deaths,
        members: vec![],
    };
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Ended(BattlefieldEnd {
            zone_id: 501,
            instance_id: 9,
            reason: 2,
            teams: [team(2, 159, 17, 19), team(3, 160, -1, -2)],
        })),
    );
    rt.lua
        .load(
            r#"
        local s=X2BattleField:GetProgressScoreInfo()
        assert(X2BattleField:IsInInstantGame() and s.myCorpsVictoryState==3)
        assert(s.myCorpsKill==65535 and s.myCorpsDeath==65534)
        assert(s.score1==16777216 and s.killScore1==9,'End does not replace global score fields with result-team score')
    "#,
        )
        .exec()?;
    Ok(())
}
