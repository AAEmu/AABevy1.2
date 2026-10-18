use super::*;

fn commands(actual: Vec<aa_net::Command>) -> Vec<(u32, aa_net::ExpeditionCommand)> {
    actual
        .into_iter()
        .map(|command| match command {
            aa_net::Command::Expedition { character_id, command } => (character_id, command),
            other => panic!("unexpected command {other:?}"),
        })
        .collect()
}

fn capture_tasks(rt: &UiRuntime) -> mlua::Result<()> {
    rt.lua
        .load(
            r#"
        GUILD_TASK_WINDOWS={};GUILD_TASK_INFO={};GUILD_TASK_TOKEN={}
        local show=HUD.expedition_show_task
        HUD.expedition_show_task=function(task,token,info) GUILD_TASK_TOKEN[task]=token;return show(task,token,info) end
        local request=X2DialogManager.RequestTaskDialog
        X2DialogManager.RequestTaskDialog=function(self,task,info,owner)
            local wnd=request(self,task,info,owner)
            if task==3 or task==26 or task==27 then GUILD_TASK_WINDOWS[task]=wnd;GUILD_TASK_INFO[task]=info end
            return wnd
        end
    "#,
        )
        .exec()
}

#[test]
fn received_identity_and_native_policy_conversion_drive_original_permission_controls() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path, _) = fixture::runtime(1280., 720.)?;
    rt.lua
        .load(
            r#"
        assert(X2Faction:GetMyExpeditionId()==900 and X2Faction:GetMyExpeditionName()=='The Silver Vale')
        assert(X2Faction:IsExpedInfoLoaded() and X2Faction:GetExpeditionMembersPerPage()==50)
        local online,total=X2Faction:GetExpeditionMemberCount();assert(online==3 and total==4)
        local all=X2Faction:GetAllExpeditionRolePolicies();assert(#all==5 and all[1][1]==0 and all[5][1]==255)
        local my=X2Faction:GetExpeditionMyRolePolicy();assert(#my==10 and my[1]==255 and my[2]=='Leader')
        for i=3,10 do assert(my[i]==true) end
        local member=X2Faction:GetExpeditionRolePolicy(0);assert(member[1]==0 and member[3]==false and member[9]==true)
        assert(X2Faction:GetExpeditionRolePolicy(1.9)[1]==1)
        assert(X2Faction:GetExpeditionRolePolicy(-0.9)[1]==0)
        for _,r in ipairs({'1',false,256,-1,1/0,0/0}) do assert(select('#',X2Faction:GetExpeditionRolePolicy(r))==0) end
        assert(select('#',X2Faction:GetExpeditionRolePolicy())==0)
        local derived=expedMgmt:GetRolePolicy('my');assert(derived[11] and not derived[12] and derived[13])
        local tab=expedMgmt.tab.window[4];assert(tab.curRoleBtn.rolePolicy[1]==255)
        assert(tab.innerWnd.delegateButton:IsVisible() and tab.innerWnd.dismissButton:IsVisible())
        assert(not tab.innerWnd.leaveButton:IsVisible())
    "#,
        )
        .exec()?;
    super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[4].roles[3]")?;
    rt.lua
        .load(
            r#"
        local tab=expedMgmt.tab.window[4]
        assert(tab.curRoleBtn.rolePolicy[1]==2 and tab.innerWnd.roleName:GetText()=='Officer')
        assert(tab.innerWnd.applyButton:IsVisible() and tab.innerWnd.chks[1]:IsEnabled())
    "#,
        )
        .exec()?;
    let mut members: Vec<_> = bridge.borrow().expedition.cache.members.values().cloned().collect();
    members.iter_mut().find(|row| row.character_id == 47).unwrap().role = 0;
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Expedition(ExpeditionEvent::Members {
            total: 4,
            expedition_id: 900,
            members,
        }),
    );
    rt.lua
        .load(
            r#"
        expedMgmt:ChangeTab(4)
        local tab=expedMgmt.tab.window[4];local my=expedMgmt:GetRolePolicy('my')
        assert(my[1]==0 and not my[3] and not my[13])
        assert(not tab.innerWnd.delegateButton:IsVisible() and not tab.innerWnd.dismissButton:IsVisible())
        assert(tab.innerWnd.leaveButton:IsVisible())
        assert(not tab.innerWnd.applyButton:IsVisible())
    "#,
        )
        .exec()?;
    dispatch(Some(&rt), &bridge, &Event::LeftWorld { target: 0 });
    rt.lua.load("assert(X2Faction:GetMyExpeditionId()==0 and X2Faction:GetMyExpeditionName()=='' and X2Faction:IsExpedInfoLoaded());assert(#X2Faction:GetAllExpeditionRolePolicies()==0 and select('#',X2Faction:GetExpeditionMyRolePolicy())==0)").exec()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn received_status_notices_follow_native_existing_new_and_name_flag_order() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path, _) = fixture::runtime(1280., 720.)?;
    rt.lua.load(r#"
        expedMgmt:Show(false);NOTICES={}
        for _,name in ipairs({'EXPEDITION_MANAGEMENT_MEMBER_STATUS_CHANGED','EXPEDITION_MANAGEMENT_UPDATED','EXPEDITION_MANAGEMENT_MEMBER_NAME_CHANGED','EXPEDITION_MANAGEMENT_MEMBERS_INFO'}) do
            local event=name
            UIParent:SetEventHandler(name,function(...) assert(select('#',...)==0);NOTICES[#NOTICES+1]=event end)
        end
    "#).exec()?;
    let mut member = bridge.borrow().expedition.cache.members[&48].clone();
    member.online = 2;
    member.name = b"Elowen\0hidden".to_vec();
    let receipt = Event::Expedition(ExpeditionEvent::MemberStatus {
        member: member.clone(),
        flag: 3,
    });
    dispatch(Some(&rt), &bridge, &receipt);
    dispatch(Some(&rt), &bridge, &receipt);
    member.character_id = 51;
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Expedition(ExpeditionEvent::MemberStatus { member, flag: 1 }),
    );
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Expedition(ExpeditionEvent::Members {
            total: 0,
            expedition_id: 1,
            members: vec![],
        }),
    );
    rt.lua
        .load(
            r#"
        assert(#NOTICES==3)
        assert(NOTICES[1]=='EXPEDITION_MANAGEMENT_MEMBER_NAME_CHANGED' and NOTICES[2]==NOTICES[1])
        assert(NOTICES[3]=='EXPEDITION_MANAGEMENT_UPDATED')
    "#,
        )
        .exec()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn original_member_rows_use_received_positions_offline_age_and_native_local_page_callbacks() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path, _) = fixture::runtime(1280., 720.)?;
    fixture::show_members(&rt)?;
    rt.lua
        .load(
            r#"
        local tab=expedMgmt.tab.window[3];local frame=tab.frame
        assert(frame:GetDataCount()==4 and not tab.refreshButton.waiting)
        local a=frame:GetDataByDataIndex(1,1);local b=frame:GetDataByDataIndex(2,1)
        local c=frame:GetDataByDataIndex(3,1);local d=frame:GetDataByDataIndex(4,1)
        assert(a[1]=='Elowen' and b[1]=='Mara' and c[1]=='Breath fixture' and d[1]=='Rydel')
        assert(a[8] and a[7] and not d[7] and d[5].day==2)
        assert(table.concat(a[5],' < ')=='Solzreed Peninsula < Nuia')
        assert(table.concat(c[5],' < ')=='Desiree Peak < Solzreed Peninsula < Nuia')
        assert(string.find(tab.numOfonlineMember:GetText(),'3/4',1,true))
        MEMBER_RESPONSES={}
        UIParent:SetEventHandler('EXPEDITION_MANAGEMENT_MEMBERS_INFO',function(total,start,rows)
            MEMBER_RESPONSES[#MEMBER_RESPONSES+1]={total,start,rows}
        end)
        X2Faction:RequestExpeditionMembers(false,1.9)
        assert(#MEMBER_RESPONSES==1 and MEMBER_RESPONSES[1][1]==3 and MEMBER_RESPONSES[1][2]==0 and #MEMBER_RESPONSES[1][3]==3)
        X2Faction:RequestExpeditionMembers(true,0);X2Faction:RequestExpeditionMembers(true,-1)
        X2Faction:RequestExpeditionMembers(nil,1);X2Faction:RequestExpeditionMembers(1,1);X2Faction:RequestExpeditionMembers(true,'1')
        assert(#MEMBER_RESPONSES==1)
        X2Faction:RequestExpeditionMembers(true,0/0)
        assert(#MEMBER_RESPONSES==2 and MEMBER_RESPONSES[2][1]==4 and MEMBER_RESPONSES[2][2]==0)
    "#,
        )
        .exec()?;
    let before = bridge.borrow().expedition.cache.clone();
    aa_ui::lua::tick(&rt.lua, 1100.);
    super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[3].refreshButton")?;
    rt.lua
        .load("assert(#MEMBER_RESPONSES==3 and MEMBER_RESPONSES[3][1]==4 and expedMgmt.tab.window[3].frame:GetDataCount()==4)")
        .exec()?;
    assert_eq!(
        before,
        bridge.borrow().expedition.cache,
        "local paging is not a wire request or a received cache mutation"
    );
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn original_permissions_and_member_invite_send_captured_requests_without_optimistic_policy_changes()
-> Result<(), Box<dyn std::error::Error>> {
    use aa_net::ExpeditionCommand as C;
    let (rt, bridge, path, player) = fixture::runtime(1280., 720.)?;
    super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[4].roles[3]")?;
    rt.lua
        .load("local p=expedMgmt.tab.window[4].innerWnd;p.chks[1]:SetChecked(false,false)")
        .exec()?;
    super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[4].innerWnd.applyButton")?;
    let requested = commands(actions::flush(&mut bridge.borrow_mut(), &player));
    assert_eq!(requested.len(), 1);
    let C::ChangePolicy { policy } = &requested[0].1 else {
        panic!("original Apply did not queue policy")
    };
    assert_eq!(
        (requested[0].0, policy.expedition_id, policy.role, policy.permissions[1]),
        (47, 900, 2, 0)
    );
    assert_eq!(
        bridge.borrow().expedition.cache.policies[&(900, 2)].permissions[1],
        1,
        "Apply is not a server acknowledgement"
    );
    super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[4].innerWnd.changeRoleName")?;
    rt.lua
        .load("expedMgmt.tab.window[4].innerWnd.editRoleName:SetText('Sentinel')")
        .exec()?;
    super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[4].innerWnd.changeComplete")?;
    let requested = commands(actions::flush(&mut bridge.borrow_mut(), &player));
    assert!(matches!(&requested[..],[(47,C::ChangePolicy{policy})] if policy.name==b"Sentinel"));
    fixture::show_members(&rt)?;
    rt.lua
        .load("expedMgmt.tab.window[3].inputName:SetText('Lyra');assert(expedMgmt.tab.window[3].inviteExped:IsEnabled())")
        .exec()?;
    super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[3].inviteExped")?;
    assert_eq!(
        commands(actions::flush(&mut bridge.borrow_mut(), &player)),
        [(47, C::Invite { name: b"Lyra".to_vec() })]
    );
    rt.lua
        .load("X2Faction:KickFromExpedition('eLoWeN');X2Faction:ChangeExpeditionMemberRole('Elowen',1)")
        .exec()?;
    assert_eq!(
        commands(actions::flush(&mut bridge.borrow_mut(), &player)),
        [
            (47, C::Kick { member_id: 48 }),
            (47, C::ChangeMemberRole { role: 1, member_id: 48 })
        ]
    );
    rt.lua.load("X2Faction:InviteToExpedition('OldActor')").exec()?;
    bridge.borrow_mut().character_id = 99;
    assert!(
        actions::flush(&mut bridge.borrow_mut(), &player).is_empty(),
        "queued UI action must not survive a changed selected character"
    );
    if path.exists() {
        std::fs::remove_file(path)?
    }
    Ok(())
}

#[test]
fn original_leave_and_dismiss_confirmations_send_only_once_after_positive_answers() -> Result<(), Box<dyn std::error::Error>> {
    use aa_net::ExpeditionCommand as C;
    let (rt, bridge, path, player) = fixture::runtime(1280., 720.)?;
    capture_tasks(&rt)?;
    for accepted in [false, true] {
        super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[4].innerWnd.dismissButton")?;
        assert!(actions::flush(&mut bridge.borrow_mut(), &player).is_empty());
        tasks::show(&rt.lua, &bridge);
        rt.lua
            .load("assert(GUILD_TASK_WINDOWS[27]:IsVisible() and GUILD_TASK_INFO[27].buttonType==0 and GUILD_TASK_INFO[27].isModal)")
            .exec()?;
        let button = if accepted {
            "GUILD_TASK_WINDOWS[27].btnOk"
        } else {
            "GUILD_TASK_WINDOWS[27].btnCancel"
        };
        super::super::expansion::fixture::click(&rt, button)?;
        let sent = commands(actions::flush(&mut bridge.borrow_mut(), &player));
        assert_eq!(sent, if accepted { vec![(47, C::Dismiss)] } else { vec![] });
        assert_eq!(
            bridge.borrow().expedition.cache.own_id,
            900,
            "confirmation is not dismissal settlement"
        );
    }
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Expedition(ExpeditionEvent::RoleChanged {
            character_id: 47,
            role: 0,
            name: b"Member".to_vec(),
        }),
    );
    rt.lua
        .load("expedMgmt:ChangeTab(4);assert(expedMgmt.tab.window[4].innerWnd.leaveButton:IsVisible())")
        .exec()?;
    for accepted in [false, true] {
        super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[4].innerWnd.leaveButton")?;
        tasks::show(&rt.lua, &bridge);
        rt.lua
            .load("assert(GUILD_TASK_WINDOWS[26]:IsVisible() and GUILD_TASK_INFO[26].isModal)")
            .exec()?;
        let button = if accepted {
            "GUILD_TASK_WINDOWS[26].btnOk"
        } else {
            "GUILD_TASK_WINDOWS[26].btnCancel"
        };
        super::super::expansion::fixture::click(&rt, button)?;
        assert_eq!(
            commands(actions::flush(&mut bridge.borrow_mut(), &player)),
            if accepted { vec![(47, C::Leave)] } else { vec![] }
        );
    }
    if path.exists() {
        std::fs::remove_file(path)?
    }
    Ok(())
}

#[test]
fn original_invitation_captures_ids_rejects_duplicates_ignores_and_stale_callbacks() -> Result<(), Box<dyn std::error::Error>> {
    use aa_net::ExpeditionCommand as C;
    let (rt, bridge, path, mut player) = fixture::runtime(1280., 720.)?;
    capture_tasks(&rt)?;
    rt.lua
        .load("expedMgmt:Show(false);X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_EXPEDITION_INVITATION,0)")
        .exec()?;
    let spawn = player.spawn.as_mut().unwrap();
    spawn.state.character.expedition_id = 0;
    spawn.unit.expedition_id = 0;
    dispatch(Some(&rt), &bridge, &Event::CharacterSelected(Box::new(spawn.state.clone())));
    dispatch(Some(&rt), &bridge, &Event::PlayerSpawn(Box::new(spawn.clone())));
    let invitation = |inviter_id, expedition_id| {
        Event::Expedition(ExpeditionEvent::Invitation {
            inviter_id,
            expedition_id,
            inviter_name: b"Elowen".to_vec(),
            expedition_name: b"Untrusted packet label".to_vec(),
        })
    };
    dispatch(Some(&rt), &bridge, &invitation(88, 900));
    rt.lua.load("assert(GUILD_TASK_WINDOWS[3]:IsVisible() and GUILD_TASK_INFO[3].isModal==false);assert(string.find(GUILD_TASK_INFO[3].content,'The Silver Vale',1,true) and not string.find(GUILD_TASK_INFO[3].content,'Untrusted',1,true))").exec()?;
    let token: u32 = rt.lua.load("return GUILD_TASK_TOKEN[3]").eval()?;
    dispatch(Some(&rt), &bridge, &invitation(89, 901));
    assert_eq!(
        commands(actions::flush(&mut bridge.borrow_mut(), &player)),
        [(
            47,
            C::Reply {
                inviter_id: 89,
                expedition_id: 901,
                join: false
            }
        )]
    );
    super::super::expansion::fixture::click(&rt, "GUILD_TASK_WINDOWS[3].btnOk")?;
    assert_eq!(
        commands(actions::flush(&mut bridge.borrow_mut(), &player)),
        [(
            47,
            C::Reply {
                inviter_id: 88,
                expedition_id: 900,
                join: true
            }
        )]
    );
    assert_eq!(
        bridge.borrow().expedition.cache.own_id,
        0,
        "accepting invitation does not acknowledge membership"
    );
    rt.lua
        .load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_EXPEDITION_INVITATION,1)")
        .exec()?;
    dispatch(Some(&rt), &bridge, &invitation(90, 902));
    assert_eq!(
        commands(actions::flush(&mut bridge.borrow_mut(), &player)),
        [(
            47,
            C::Reply {
                inviter_id: 90,
                expedition_id: 902,
                join: false
            }
        )]
    );
    rt.lua
        .load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_EXPEDITION_INVITATION,0)")
        .exec()?;
    dispatch(Some(&rt), &bridge, &invitation(91, 903));
    rt.lua.load(format!("HUD.expedition_choose({token},true)")).exec()?;
    assert!(actions::flush(&mut bridge.borrow_mut(), &player).is_empty());
    super::super::expansion::fixture::click(&rt, "GUILD_TASK_WINDOWS[3].btnCancel")?;
    assert_eq!(
        commands(actions::flush(&mut bridge.borrow_mut(), &player)),
        [(
            47,
            C::Reply {
                inviter_id: 91,
                expedition_id: 903,
                join: false
            }
        )]
    );
    dispatch(Some(&rt), &bridge, &invitation(92, 904));
    dispatch(Some(&rt), &bridge, &Event::LeftWorld { target: 0 });
    rt.lua.load("assert(not GUILD_TASK_WINDOWS[3]:IsVisible())").exec()?;
    assert!(actions::flush(&mut bridge.borrow_mut(), &player).is_empty());
    if path.exists() {
        std::fs::remove_file(path)?
    }
    Ok(())
}

#[test]
fn original_leadership_transfer_uses_received_dominion_gate_and_received_owner_settlement() -> Result<(), Box<dyn std::error::Error>> {
    use aa_net::{DominionData, DominionEvent as D, DominionSiegeTimers, ExpeditionCommand as C};
    let (rt, bridge, path, player) = fixture::runtime(1280., 720.)?;
    let transfer = || -> Result<(), Box<dyn std::error::Error>> {
        super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[4].innerWnd.delegateButton")?;
        rt.lua
            .load("expedMgmt.tab.window[4].delegateWindow.inputName:SetText('Elowen')")
            .exec()?;
        super::super::expansion::fixture::click(&rt, "expedMgmt.tab.window[4].delegateWindow.ok")?;
        Ok(())
    };
    transfer()?;
    assert_eq!(
        commands(actions::flush(&mut bridge.borrow_mut(), &player)),
        [(47, C::ChangeOwner { member_id: 48 })]
    );
    assert_eq!(
        bridge.borrow().expedition.cache.factions[&900].owner_id,
        47,
        "a sent transfer is not settlement"
    );
    let mut data = DominionData {
        zone_group_id: 5,
        siege_timers: DominionSiegeTimers {
            siege_period: 4,
            ..default()
        },
        ..default()
    };
    data.siege_timers.defender.expedition_id = 900;
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Dominion(D::Data {
            data: data.clone(),
            newly_declared: 0,
            final_data_by_request: 1,
        }),
    );
    transfer()?;
    assert!(
        actions::flush(&mut bridge.borrow_mut(), &player).is_empty(),
        "actual active siege participant blocks transfer"
    );
    data.siege_timers.siege_period = 3;
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Dominion(D::Data {
            data: data.clone(),
            newly_declared: 0,
            final_data_by_request: 1,
        }),
    );
    transfer()?;
    assert_eq!(
        commands(actions::flush(&mut bridge.borrow_mut(), &player)),
        [(47, C::ChangeOwner { member_id: 48 })]
    );
    dispatch(
        Some(&rt),
        &bridge,
        &Event::UnhandledGamePacket {
            level: aa_net::game::LEVEL_COMPRESSED,
            opcode: 0xffff,
            len: 10,
        },
    );
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Dominion(D::Data {
            data,
            newly_declared: 1,
            final_data_by_request: 1,
        }),
    );
    transfer()?;
    assert!(
        actions::flush(&mut bridge.borrow_mut(), &player).is_empty(),
        "one row/flags must not fabricate complete eligibility after an abandoned compressed tail"
    );
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Expedition(ExpeditionEvent::OwnerChanged {
            old_owner_id: 47,
            new_owner_id: 48,
            name: b"Elowen".to_vec(),
        }),
    );
    assert_eq!(bridge.borrow().expedition.cache.factions[&900].owner_id, 48);
    rt.lua.load("assert(X2Faction:GetExpeditionMyRolePolicy()[1]==0);expedMgmt:ChangeTab(4);assert(expedMgmt.tab.window[4].innerWnd.leaveButton:IsVisible() and not expedMgmt.tab.window[4].innerWnd.delegateButton:IsVisible())").exec()?;
    if path.exists() {
        std::fs::remove_file(path)?
    }
    Ok(())
}
