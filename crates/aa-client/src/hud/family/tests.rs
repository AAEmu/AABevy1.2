use super::*;
use aa_net::family::{FamilyEvent as E, FamilyInvitation, FamilyMember, FamilyRoster};

fn assert_commands<const N: usize>(actual: Vec<aa_net::Command>, expected: [aa_net::Command; N]) {
    let family = |command| match command {
        aa_net::Command::Family { character_id, command } => (character_id, command),
        other => panic!("unexpected non-family command: {other:?}"),
    };
    assert_eq!(
        actual.into_iter().map(family).collect::<Vec<_>>(),
        expected.into_iter().map(family).collect::<Vec<_>>()
    );
}

fn member(id: u32, name: &str, role: u8, online: bool) -> FamilyMember {
    FamilyMember {
        character_id: id,
        name: name.as_bytes().to_vec(),
        role,
        online,
        title: b"Companion".to_vec(),
    }
}
fn roster(id: u32) -> FamilyRoster {
    FamilyRoster {
        family_id: id,
        members: vec![member(47, "Self", 2, true), member(48, "Friend", 0, false), member(49, "Third", 0, true)],
    }
}
fn setup() -> Result<(UiRuntime, Shared, aa_net::state::Player), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", 1920., 1080.)?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(super::super::api::HudData::default()));
    bridge.borrow_mut().character_id = 47;
    bridge.borrow_mut().snapshot.unit_id = "100".into();
    super::super::api::install(&mut rt, &bridge)?;
    let _ = crate::settings::isolated_bindings(&rt)?;
    rt.lua.load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_FAMILY_INVITATION,0)").exec()?;
    let player = aa_net::state::Player {
        spawn: Some(super::super::expansion::fixture::spawn()),
    };
    dispatch(None, &bridge, &Event::PlayerSpawn(Box::new(player.spawn.as_ref().unwrap().clone())));
    Ok((rt, bridge, player))
}

#[test]
fn ui_cache_caps_and_cstring_rows_do_not_infer_own_family_from_unrelated_rosters() {
    let mut view = state::View::default();
    view.actor(47, 100, 0);
    let mut received = roster(12);
    received.members.remove(0);
    assert!(view.receipt(&E::Described(received)).is_empty());
    assert_eq!(view.family, 0);
    let mut received = roster(13);
    received.members[1].name = b"Case\0not-part-of-name".to_vec();
    received.members[1].title = vec![b'T'; 110];
    received.members.extend((50..59).map(|id| member(id, "Other", 0, true)));
    view.receipt(&E::Described(received));
    assert_eq!(view.family, 13);
    assert_eq!(view.roster().unwrap().members.len(), 8);
    assert_eq!(view.member(b"Case\0ignored").unwrap().title.len(), 104);
    assert!(view.member(b"case").is_none());
    assert!(view.is_owner(), "native owner means nonzero role, including role2");
    let mut omitted = roster(13);
    omitted.members.remove(0);
    view.receipt(&E::Described(omitted));
    assert_eq!(view.family, 13, "full replacement does not clear omitted actor IDs");
    assert!(!view.is_owner());
}

#[test]
fn native_notices_preserve_duplicates_owner_order_first_removal_and_dissolution() {
    let mut view = state::View::default();
    view.actor(47, 100, 0);
    let mut received = roster(12);
    received.members[2].role = 3;
    assert_eq!(
        view.receipt(&E::Created(received.clone())),
        [state::Notice::Added(b"Self".to_vec(), b"Friend".to_vec(), b"Companion".to_vec())]
    );
    let notice = E::MemberOnline {
        family_id: 12,
        character_id: 48,
        online: false,
    };
    for _ in 0..2 {
        assert_eq!(view.receipt(&notice), [state::Notice::Online(b"Friend".to_vec(), false)]);
    }
    assert_eq!(
        view.receipt(&E::TitleChanged {
            family_id: 12,
            character_id: 48,
            title: b"Brother".to_vec()
        }),
        [state::Notice::Added(b"Third".to_vec(), b"Friend".to_vec(), b"Brother".to_vec())]
    );
    received.members = vec![member(47, "First", 1, true), member(47, "Last", 0, false), member(48, "Friend", 0, true)];
    view.receipt(&E::Described(received));
    assert_eq!(
        view.receipt(&E::OwnerChanged {
            family_id: 12,
            character_id: 47
        }),
        [state::Notice::Owner(b"Last".to_vec())]
    );
    assert!(view.roster().unwrap().members[..2].iter().all(|m| m.role == 1 && m.title.is_empty()));
    assert_eq!(
        view.receipt(&E::MemberRemoved {
            family_id: 12,
            character_id: 47,
            kicked: true
        }),
        [state::Notice::Left(b"First".to_vec(), true)]
    );
    assert_eq!(
        view.family, 0,
        "the first removed row's callback clears the actor ID even if a duplicate remains"
    );
    assert_eq!(
        view.receipt(&E::MemberRemoved {
            family_id: 12,
            character_id: 47,
            kicked: false
        }),
        [state::Notice::Left(b"Last".to_vec(), false)]
    );
    assert_eq!(view.receipt(&E::Removed { family_id: 999 }), [state::Notice::Removed]);
}

#[test]
fn native_getters_and_action_feature_gate_keep_names_roles_and_authority_received_only() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, player) = setup()?;
    rt.lua
        .load("assert(not X2Family:IsFamily() and #X2Family:GetMembers(true)==0);assert(select('#',X2Family:GetMembers())==0)")
        .exec()?;
    dispatch(Some(&rt), &bridge, &Event::Family(E::Described(roster(12))));
    rt.lua
        .load(
            r#"
      assert(X2Family:IsFamily() and X2Family:IsOwner() and X2Family:IsMyFamily('Friend') and not X2Family:IsMyFamily('friend'))
      local m=X2Family:GetMembers(false);assert(#m==2 and m[1][1]=='Self' and m[1][3]==2 and m[1][4]==true)
      local on,total=X2Family:GetMemberCount();assert(on==2 and total==3 and #X2Family:GetMembers(true)==3)
      X2Family:Invite('New','Title');X2Family:Leave();X2Family:Kick('Friend');X2Family:ChangeTitle('Friend','New')
      X2Family:ChangeOwner('Friend')
    "#,
        )
        .exec()?;
    assert_commands(
        flush(&mut bridge.borrow_mut(), &player),
        [aa_net::Command::Family {
            character_id: 47,
            command: FamilyCommand::ChangeOwner { member_id: 48 },
        }],
    );
    bridge.borrow_mut().family.changes = Some(true);
    rt.lua.load("X2Family:Invite('New\\000tail','Sibling');X2Family:Leave();X2Family:Kick('friend');X2Family:Kick('Friend');X2Family:ChangeTitle('Friend','New');X2Family:ChangeOwner('Self')").exec()?;
    let commands = flush(&mut bridge.borrow_mut(), &player);
    assert_eq!(commands.len(), 4);
    assert!(matches!(&commands[0], aa_net::Command::Family { command: FamilyCommand::Invite { name, .. }, .. } if name==b"New"));
    assert_eq!(bridge.borrow().family.state.roster().unwrap().members.len(), 3);
    rt.lua.load("X2Family:Kick('Friend')").exec()?;
    bridge.borrow_mut().character_id = 999;
    assert!(
        flush(&mut bridge.borrow_mut(), &player).is_empty(),
        "queued actions may not move to another selected character"
    );
    Ok(())
}

#[test]
fn callbacks_observe_received_mutation_and_invites_keep_captured_bytes_single_answers_and_option74() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, player) = setup()?;
    rt.lua
        .load(
            r#"
      FAMILY_CALLBACKS=0
      UIParent:SetEventHandler('FAMILY_MEMBER_ONLINE',function(name,online)
        local on,total=X2Family:GetMemberCount();assert(name=='Friend' and online and on==3 and total==3)
        FAMILY_CALLBACKS=FAMILY_CALLBACKS+1
      end)
      HUD.family_show_invitation=function(token,info)FAMILY_TOKEN=token;FAMILY_INFO=info end
    "#,
        )
        .exec()?;
    dispatch(Some(&rt), &bridge, &Event::Family(E::Described(roster(12))));
    for _ in 0..2 {
        dispatch(
            Some(&rt),
            &bridge,
            &Event::Family(E::MemberOnline {
                family_id: 12,
                character_id: 48,
                online: true,
            }),
        );
    }
    rt.lua.load("assert(FAMILY_CALLBACKS==2)").exec()?;
    let invite = FamilyInvitation {
        inviter_id: 88,
        inviter_name: b"Inviter\0suffix".to_vec(),
        family_id: 24,
        role: b"Sibling\0suffix".to_vec(),
    };
    dispatch(Some(&rt), &bridge, &Event::Family(E::Invitation(invite.clone())));
    rt.lua.load("assert(FAMILY_INFO.invitorName=='Inviter' and FAMILY_INFO.familyTitle=='Sibling' and FAMILY_INFO.buttonType==0 and FAMILY_INFO.isModal==false);OLD_FAMILY_TOKEN=FAMILY_TOKEN").exec()?;
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Family(E::Invitation(FamilyInvitation {
            inviter_id: 89,
            ..invite.clone()
        })),
    );
    rt.lua
        .load("HUD.family_choose(FAMILY_TOKEN,true);HUD.family_choose(FAMILY_TOKEN,false)")
        .exec()?;
    let commands = flush(&mut bridge.borrow_mut(), &player);
    assert_commands(
        commands,
        [
            aa_net::Command::Family {
                character_id: 47,
                command: FamilyCommand::Reply {
                    inviter_id: 89,
                    join: false,
                    role: b"Sibling".to_vec(),
                },
            },
            aa_net::Command::Family {
                character_id: 47,
                command: FamilyCommand::Reply {
                    inviter_id: 88,
                    join: true,
                    role: b"Sibling".to_vec(),
                },
            },
        ],
    );
    rt.lua.load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_FAMILY_INVITATION,1)").exec()?;
    dispatch(Some(&rt), &bridge, &Event::Family(E::Invitation(invite.clone())));
    assert!(matches!(
        &flush(&mut bridge.borrow_mut(), &player)[..],
        [aa_net::Command::Family {
            command: FamilyCommand::Reply { join: false, .. },
            ..
        }]
    ));
    rt.lua.load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_FAMILY_INVITATION,0)").exec()?;
    dispatch(None, &bridge, &Event::LeftWorld { target: 1 });
    dispatch(None, &bridge, &Event::PlayerSpawn(Box::new(player.spawn.as_ref().unwrap().clone())));
    dispatch(Some(&rt), &bridge, &Event::Family(E::Invitation(invite)));
    rt.lua
        .load("assert(FAMILY_TOKEN~=OLD_FAMILY_TOKEN);HUD.family_choose(OLD_FAMILY_TOKEN,true)")
        .exec()?;
    assert!(flush(&mut bridge.borrow_mut(), &player).is_empty());
    Ok(())
}

#[test]
#[ignore = "loads the original family relationship tab, input buttons and type4 invitation dialog"]
fn original_family_tab_and_captured_invitation_buttons_use_received_data() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(1920., 1080.)?;
    rt.lua.load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_FAMILY_INVITATION,0)").exec()?;
    let player = aa_net::state::Player {
        spawn: Some(super::super::expansion::fixture::spawn()),
    };
    bridge.borrow_mut().character_id = 47;
    bridge.borrow_mut().snapshot.name = "Self".into();
    super::super::api::push_changes(&rt, &bridge);
    dispatch(Some(&rt), &bridge, &Event::PlayerSpawn(Box::new(player.spawn.as_ref().unwrap().clone())));
    bridge.borrow_mut().family.changes = Some(true);
    dispatch(Some(&rt), &bridge, &Event::Family(E::Described(roster(12))));
    rt.lua
        .load(
            r#"
      ToggleRelationshipUI(true);relationshipFrame.tab:SelectTab(relationshipFrame.tab.familyTabIdx)
      FAMILY_TAB=relationshipFrame.tab.window[3];FAMILY_TAB:FillMemberData()
      assert(FAMILY_TAB.frame.dataCount==2)
      FAMILY_TAB.showOffMember:SetChecked(true,false);FAMILY_TAB:FillMemberData();assert(FAMILY_TAB.frame.dataCount==3)
      FAMILY_TAB.inputName:SetText('Newfamily');FAMILY_TAB.inputTitle:SetText('Sibling')
      assert(FAMILY_TAB.addFamilyButton:IsEnabled())
      local original=X2DialogManager.RequestTaskDialog
      X2DialogManager.RequestTaskDialog=function(self,task,info,owner)
        local wnd=original(self,task,info,owner);if task==DLG_TASK_JOIN_FAMILY then FAMILY_DIALOG=wnd end;return wnd
      end
    "#,
        )
        .exec()?;
    super::super::expansion::fixture::click(&rt, "FAMILY_TAB.addFamilyButton")?;
    assert_commands(
        flush(&mut bridge.borrow_mut(), &player),
        [aa_net::Command::Family {
            character_id: 47,
            command: FamilyCommand::Invite {
                name: b"Newfamily".to_vec(),
                title: b"Sibling".to_vec(),
            },
        }],
    );
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Family(E::Invitation(FamilyInvitation {
            inviter_id: 88,
            inviter_name: b"Inviter".to_vec(),
            family_id: 24,
            role: b"Sibling".to_vec(),
        })),
    );
    rt.lua
        .load("assert(FAMILY_DIALOG:IsVisible() and FAMILY_DIALOG.btnOk:IsVisible() and FAMILY_DIALOG.btnCancel:IsVisible())")
        .exec()?;
    super::super::expansion::fixture::click(&rt, "FAMILY_DIALOG.btnOk")?;
    assert_commands(
        flush(&mut bridge.borrow_mut(), &player),
        [aa_net::Command::Family {
            character_id: 47,
            command: FamilyCommand::Reply {
                inviter_id: 88,
                join: true,
                role: b"Sibling".to_vec(),
            },
        }],
    );
    rt.lua
        .load(
            r#"
      local create=CreateEmptyWindow
      CreateEmptyWindow=function(id,parent,...)
        local wnd=create(id,parent,...);if id=='popupMenu' then FAMILY_POPUP=wnd end;return wnd
      end
      local request=X2DialogManager.RequestDefaultDialog
      X2DialogManager.RequestDefaultDialog=function(self,fn,owner)
        local wnd=request(self,fn,owner);FAMILY_EDIT=wnd;return wnd
      end
      function FAMILY_SELECT_POPUP(name,text)
        for i=1,FAMILY_TAB.frame.dataCount do
          if FAMILY_TAB.frame:GetDataByViewIndex(i,1)[1]==name then FAMILY_TAB.frame.listCtrl.items[i]:OnClick('RightButton');break end
        end
        FAMILY_CHOICE=nil
        for _,button in ipairs(FAMILY_POPUP.buttons) do
          if button:GetText()==text then FAMILY_CHOICE=button;break end
        end
        assert(FAMILY_CHOICE,'original family popup action missing: '..text)
      end
      FAMILY_SELECT_POPUP('Friend',locale.family.kick_member)
    "#,
        )
        .exec()?;
    super::super::expansion::fixture::click(&rt, "FAMILY_CHOICE")?;
    rt.lua.load("FAMILY_SELECT_POPUP('Friend',locale.family.change_owner)").exec()?;
    super::super::expansion::fixture::click(&rt, "FAMILY_CHOICE")?;
    rt.lua.load("FAMILY_SELECT_POPUP('Friend',locale.family.changeTitle)").exec()?;
    super::super::expansion::fixture::click(&rt, "FAMILY_CHOICE")?;
    rt.lua.load("assert(FAMILY_EDIT:IsVisible());FAMILY_EDIT.editbox:SetText('Beloved')").exec()?;
    super::super::expansion::fixture::click(&rt, "FAMILY_EDIT.btnOk")?;
    assert_commands(
        flush(&mut bridge.borrow_mut(), &player),
        [
            aa_net::Command::Family {
                character_id: 47,
                command: FamilyCommand::Kick { member_id: 48 },
            },
            aa_net::Command::Family {
                character_id: 47,
                command: FamilyCommand::ChangeOwner { member_id: 48 },
            },
            aa_net::Command::Family {
                character_id: 47,
                command: FamilyCommand::ChangeTitle {
                    member_id: 48,
                    title: b"Beloved".to_vec(),
                },
            },
        ],
    );
    assert!(bridge.borrow().family.state.is_owner(), "requesting ownership transfer does not acknowledge it");
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Family(E::OwnerChanged {
            family_id: 12,
            character_id: 48,
        }),
    );
    rt.lua.load("FAMILY_SELECT_POPUP('Self',locale.family.leave_family)").exec()?;
    super::super::expansion::fixture::click(&rt, "FAMILY_CHOICE")?;
    assert_commands(
        flush(&mut bridge.borrow_mut(), &player),
        [aa_net::Command::Family {
            character_id: 47,
            command: FamilyCommand::Leave,
        }],
    );
    for direct_cancel in [false, true] {
        dispatch(
            Some(&rt),
            &bridge,
            &Event::Family(E::Invitation(FamilyInvitation {
                inviter_id: 89,
                inviter_name: b"Inviter".to_vec(),
                family_id: 24,
                role: b"Sibling".to_vec(),
            })),
        );
        if direct_cancel {
            rt.lua.load("X2DialogManager:OnCancel(FAMILY_DIALOG:GetId())").exec()?;
        } else {
            super::super::expansion::fixture::click(&rt, "FAMILY_DIALOG.btnCancel")?;
        }
        assert_commands(
            flush(&mut bridge.borrow_mut(), &player),
            [aa_net::Command::Family {
                character_id: 47,
                command: FamilyCommand::Reply {
                    inviter_id: 89,
                    join: false,
                    role: b"Sibling".to_vec(),
                },
            }],
        );
    }
    assert!(rt.errors.is_empty());
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
