use super::*;
use aa_net::battlefield::{BattlefieldInvitation, BattlefieldJoined};

fn admit(rt: &UiRuntime, bridge: &Shared) {
    let command = BattlefieldCommand::Apply {
        battlefield_id: 1,
        corps: 2,
    };
    dispatch(
        rt,
        bridge,
        &Event::Battlefield(BattlefieldEvent::Requested(command)),
    );
    dispatch(
        rt,
        bridge,
        &Event::Battlefield(BattlefieldEvent::Applied {
            battlefield_id: 1,
            corps: 2,
            error: 0,
        }),
    );
}

fn invite(qualified_id: u64) -> BattlefieldInvitation {
    BattlefieldInvitation {
        zone_id: 501,
        instance_id: 9,
        ruleset_id: 27,
        corps: 1,
        qualified_id,
    }
}

fn capture(rt: &UiRuntime) -> mlua::Result<()> {
    rt.lua
        .load(
            r#"
        local show=HUD.battlefield_show_invitation
        HUD.battlefield_show_invitation=function(token,info)
            BATTLE_TOKEN=token
            return show(token,info)
        end
        local request=X2DialogManager.RequestTaskDialog
        X2DialogManager.RequestTaskDialog=function(self,task,info,owner)
            local wnd=request(self,task,info,owner)
            if task==DLG_TASK_JOIN_BATTLE_FIELD then BATTLE_DIALOG=wnd;BATTLE_INFO=info end
            return wnd
        end
    "#,
        )
        .exec()
}

#[test]
fn retained_invitation_tokens_reject_old_choices_and_full_qualifier_survives_join_continuation()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = super::tests::setup()?;
    capture(&rt)?;
    admit(&rt, &bridge);
    let first = invite(9_007_199_254_740_993);
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Invited(first)),
    );
    rt.lua
        .load("OLD_BATTLE_TOKEN=BATTLE_TOKEN;HUD.battlefield_choose(BATTLE_TOKEN,2)")
        .exec()?;
    let command = BattlefieldCommand::RespondToInvitation {
        invitation: first,
        accept: false,
    };
    assert_eq!(flush(&mut bridge.borrow_mut()), [command]);
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Requested(command)),
    );
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Applied {
            battlefield_id: 1,
            corps: 2,
            error: 0,
        }),
    );
    let second = invite(first.qualified_id + 2);
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Invited(second)),
    );
    rt.lua
        .load("assert(BATTLE_TOKEN~=OLD_BATTLE_TOKEN);HUD.battlefield_choose(OLD_BATTLE_TOKEN,1)")
        .exec()?;
    assert!(flush(&mut bridge.borrow_mut()).is_empty());
    rt.lua
        .load("HUD.battlefield_choose(BATTLE_TOKEN,1);HUD.battlefield_choose(BATTLE_TOKEN,2);assert(not X2BattleField:IsInInstantGame())")
        .exec()?;
    let command = BattlefieldCommand::RespondToInvitation {
        invitation: second,
        accept: true,
    };
    assert_eq!(flush(&mut bridge.borrow_mut()), [command]);
    assert!(
        bridge
            .borrow()
            .battlefield
            .received
            .response_pending
            .is_none(),
        "response write is not a received join"
    );
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Requested(command)),
    );
    entered_world(&rt, &bridge);
    assert_eq!(
        flush(&mut bridge.borrow_mut()),
        [BattlefieldCommand::EnteredWorld { invitation: second }]
    );
    rt.lua.load("assert(not X2BattleField:IsInInstantGame());entered=0;UIParent:SetEventHandler('ENTERED_INSTANT_GAME_ZONE',function(...)assert(select('#',...)==0 and X2BattleField:IsInInstantGame());entered=entered+1 end)").exec()?;
    let mut joined = BattlefieldJoined {
        zone_id: 501,
        instance_id: 10,
        corps: 1,
        victory_score: 0,
        victory_kill_count: 0,
        victory_kill_corps_head: 13353,
        phase_times_ms: [1000, 2000, 3000],
        factions: [159, 160],
        killstreak_skills: [0; 24],
    };
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Joined(joined)),
    );
    rt.lua
        .load("assert(entered==0 and not X2BattleField:IsInInstantGame())")
        .exec()?;
    joined.instance_id = 9;
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Joined(joined)),
    );
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Joined(joined)),
    );
    rt.lua
        .load("assert(entered==1 and X2BattleField:IsInInstantGame());X2BattleField:CancelInstanceGame()")
        .exec()?;
    assert!(
        flush(&mut bridge.borrow_mut()).is_empty(),
        "native cancellation is inactive in states above4"
    );
    Ok(())
}

#[test]
fn recruiter_service_uses_actual_received_skill_npc_context_and_native_minimum_level()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = super::tests::setup()?;
    rt.lua.load("recruiter_calls=0;UIParent:SetEventHandler('NPC_INTERACTION_START',function(kind,extra,id) assert(kind=='recruiter' and id=='201');recruiter_calls=recruiter_calls+1 end)").exec()?;
    let mut units = aa_net::state::Units::default();
    let mut unit = super::super::expansion::fixture::spawn().unit;
    unit.obj_id = 201;
    unit.kind = aa_net::model::UnitKind::Npc {
        template_id: 3,
        owner_id: 0,
    };
    units.by_id.insert(201, unit);
    {
        let mut data = bridge.borrow_mut();
        data.snapshot.unit_id = "100".into();
        data.dialog.npc = Some((201, 3));
        data.panels.target = Some(super::super::panels::TargetSnap {
            obj_id: 201,
            npc_type: Some(3),
            detail: "npc",
            ..default()
        });
        data.target_pos = Some(Vec3::X);
    }
    let player = aa_net::state::Player {
        spawn: Some(super::super::expansion::fixture::spawn()),
    };
    let selected = super::super::dialog::fixture::selected(
        &mut bridge.borrow_mut(),
        &player,
        units.get(201).unwrap(),
        12660,
    );
    start_selected(&rt, &bridge, selected, &units);
    bridge.borrow_mut().battlefield.level = Some(34);
    start_selected(&rt, &bridge, selected, &units);
    bridge.borrow_mut().battlefield.level = Some(45);
    units.by_id.get_mut(&201).unwrap().kind = aa_net::model::UnitKind::Mate {
        tl_id: 1,
        template_id: 3,
        owner_id: 47,
    };
    start_selected(&rt, &bridge, selected, &units);
    rt.lua.load("assert(recruiter_calls==1)").exec()?;
    Ok(())
}

#[test]
#[ignore = "loads original three-button battlefield invitation task and physical pointer controls"]
fn original_invitation_next_reject_escape_timeout_and_accept_controls()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::fixture::runtime(1920., 1080.)?;
    capture(&rt)?;
    rt.lua
        .load("X2BattleField:RequestInstanceGame(3,2)")
        .exec()?;
    flush(&mut bridge.borrow_mut());
    admit(&rt, &bridge);
    let invitation = invite(9_007_199_254_740_993);
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Invited(invitation)),
    );
    rt.lua.load("assert(BATTLE_DIALOG:IsVisible() and BATTLE_DIALOG.btnSecond:IsVisible() and BATTLE_DIALOG.btnThird:IsVisible());assert(not BATTLE_DIALOG.btnCancel:IsVisible());assert(BATTLE_INFO.buttonType==DBT_CUSTOM and BATTLE_INFO.isModal==false);assert(#BATTLE_INFO.name>0 and #BATTLE_INFO.corpsName>0)").exec()?;
    crate::hud::expansion::fixture::click(&rt, "BATTLE_DIALOG.btnSecond")?;
    let command = BattlefieldCommand::RespondToInvitation {
        invitation,
        accept: false,
    };
    assert_eq!(flush(&mut bridge.borrow_mut()), [command]);
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Requested(command)),
    );
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Applied {
            battlefield_id: 1,
            corps: 2,
            error: 0,
        }),
    );
    aa_ui::lua::tick(&rt.lua, 300.);
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Invited(invitation)),
    );
    rt.lua
        .load("X2DialogManager:OnCancel(BATTLE_DIALOG:GetId())")
        .exec()?;
    assert!(
        flush(&mut bridge.borrow_mut()).is_empty(),
        "native ESC removes the task without a queue response"
    );
    assert_eq!(
        bridge.borrow().battlefield.received.invitation,
        Some(invitation)
    );
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::InviteTimedOut {
            zone_id: 501,
            instance_id: 9,
            ruleset_id: 27,
            corps: 1,
        }),
    );
    rt.lua
        .load("assert(not BATTLE_DIALOG:IsVisible())")
        .exec()?;
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Invited(invitation)),
    );
    crate::hud::expansion::fixture::click(&rt, "BATTLE_DIALOG.btnThird")?;
    assert_eq!(
        flush(&mut bridge.borrow_mut()),
        [BattlefieldCommand::Cancel]
    );
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Cancelled { raw: 0 }),
    );
    aa_ui::lua::tick(&rt.lua, 300.);
    admit(&rt, &bridge);
    dispatch(
        &rt,
        &bridge,
        &Event::Battlefield(BattlefieldEvent::Invited(invitation)),
    );
    crate::hud::expansion::fixture::click(&rt, "BATTLE_DIALOG.btnOk")?;
    assert_eq!(
        flush(&mut bridge.borrow_mut()),
        [BattlefieldCommand::RespondToInvitation {
            invitation,
            accept: true
        }]
    );
    assert!(bridge.borrow().battlefield.received.joined.is_none());
    drop(rt);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
