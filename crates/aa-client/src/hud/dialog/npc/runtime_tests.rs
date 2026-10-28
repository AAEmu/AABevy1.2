//! Original UI plus all six actual service systems; no server or graphics device.
use super::fixture::*;
use super::*;
use crate::hud;

#[test]
#[ignore = "original UI/catalog and all six service plugins, CPU only; no network/GPU"]
fn actual_six_service_systems_dispatch_only_the_received_first_action_and_preserve_empty_discovery()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(#NPC_KINDS==0 and NPC_ENDS==0)")
        .exec()?;
    app.world_mut().write_message(NetEvent(reply(
        A,
        &[12087, 13238, 12083, 12195, 13736, 12660, 22780],
        true,
    )));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: true
        }]
    );
    let scope = active_scope(&shared.borrow()).unwrap();
    app.world().non_send::<UiRuntime>().lua.load("assert(#NPC_KINDS==1 and NPC_KINDS[1]=='store' and SALON_CALLS==0 and NPC_ENDS==1);assert(IsActivatedStore())").exec()?;
    // A duplicate receipt is still delivered, but does not dispatch another action.
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12083, 22780], true)));
    app.update();
    assert!(interactions(&drain(&mut app)).is_empty());
    query(&mut app, &shared, B);
    assert!(scope_current(&shared.borrow(), scope));
    app.world_mut().write_message(NetEvent(reply(B, &[], true)));
    app.update();
    assert!(interactions(&drain(&mut app)).is_empty());
    assert!(scope_current(&shared.borrow(), scope));
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(IsActivatedStore() and #NPC_KINDS==1 and NPC_ENDS==1)")
        .exec()?;
    // False flags gate local quest augmentation, not the received bank action.
    query(&mut app, &shared, B);
    app.world_mut()
        .write_message(NetEvent(reply(B, &[13238, 12083, 22780], false)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [
            Interact::NpcEnd { obj_id: A },
            Interact::Npc {
                obj_id: B,
                target_changed: true
            }
        ]
    );
    assert!(!scope_current(&shared.borrow(), scope));
    app.world().non_send::<UiRuntime>().lua.load("assert(#NPC_KINDS==2 and NPC_KINDS[2]=='bank' and SALON_CALLS==0 and NPC_ENDS==2);assert(IsActivatedBank())").exec()?;
    // Final Units still contains the reused ID; ordered tracking must retire it.
    query(&mut app, &shared, A);
    let unit = app.world().resource::<Units>().get(A).unwrap().clone();
    for event in [
        reply(A, &[12083, 22780], true),
        Event::UnitsRemoved(vec![A]),
        Event::UnitSpawned(Box::new(unit)),
    ] {
        app.world_mut().resource_mut::<Units>().apply(&event);
        app.world_mut().write_message(NetEvent(event));
    }
    app.update();
    assert!(interactions(&drain(&mut app)).is_empty());
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(#NPC_KINDS==2 and SALON_CALLS==0)")
        .exec()?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original NPC default/notice dialogs and exact-owner cleanup; CPU only"]
fn original_end_cleans_only_the_stable_and_guild_owner_dialogs_without_accepting_them()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    let lua = &app.world().non_send::<UiRuntime>().lua;
    lua.load(r#"
      UNRELATED_OWNER=UIParent:CreateWidget('window','npcUnrelatedOwner','UIParent')
      UNRELATED_OWNER:Show(true)
      UNRELATED_DIALOG=X2DialogManager:RequestDefaultDialog(function(w) w:SetTitle('Unrelated') end,UNRELATED_OWNER:GetId())
      NPC_ACCEPTS=0;PRIOR_OWNER_HIDES=0
      local requestNotice=X2DialogManager.RequestNoticeDialog
      X2DialogManager.RequestNoticeDialog=function(self,fn,owner)
        local dialog=requestNotice(self,fn,owner)
        if owner==GetStablerMessageBoxHandleWindow():GetId() then
          STABLE_DIALOG=dialog
          local accept=dialog.OkProc
          function dialog:OkProc(...) NPC_ACCEPTS=NPC_ACCEPTS+1;if accept then return accept(self,...) end end
        end
        return dialog
      end
      local requestDefault=X2DialogManager.RequestDefaultDialog
      X2DialogManager.RequestDefaultDialog=function(self,fn,owner)
        local dialog=requestDefault(self,fn,owner)
        if owner==GetCreateExpeditionMessageBoxHandleWindow():GetId() then GUILD_DIALOG=dialog end
        return dialog
      end
      GetStablerMessageBoxHandleWindow():SetHandler('OnHide',function()
        PRIOR_OWNER_HIDES=PRIOR_OWNER_HIDES+1
        assert(not STABLE_DIALOG:IsVisible(),'native owner deletion must precede prior Lua OnHide')
        assert(UNRELATED_DIALOG:IsVisible())
      end)
    "#).exec()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12195], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: true
        }]
    );
    app.world().non_send::<UiRuntime>().lua.load("assert(STABLE_DIALOG and STABLE_DIALOG:IsVisible());X2Interaction:CancelNPCInteraction()").exec()?;
    assert!(
        active_scope(&shared.borrow()).is_none(),
        "revocation precedes callback reentry"
    );
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::NpcEnd { obj_id: A }]
    );
    app.world().non_send::<UiRuntime>().lua.load("assert(not STABLE_DIALOG:IsVisible() and UNRELATED_DIALOG:IsVisible());assert(NPC_ACCEPTS==0 and PRIOR_OWNER_HIDES==1 and NPC_ENDS==2);X2Interaction:CancelNPCInteraction()").exec()?;
    app.update();
    assert!(interactions(&drain(&mut app)).is_empty());
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(NPC_ENDS==2 and PRIOR_OWNER_HIDES==1)")
        .exec()?;
    query(&mut app, &shared, B);
    app.world_mut()
        .write_message(NetEvent(reply(B, &[13736], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: B,
            target_changed: true
        }]
    );
    app.world().non_send::<UiRuntime>().lua.load("assert(GUILD_DIALOG and GUILD_DIALOG:IsVisible());X2Interaction:CancelNPCInteraction()").exec()?;
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::NpcEnd { obj_id: B }]
    );
    app.world().non_send::<UiRuntime>().lua.load("assert(not GUILD_DIALOG:IsVisible() and UNRELATED_DIALOG:IsVisible() and NPC_ACCEPTS==0)").exec()?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original salon task decorator and captured confirmation; CPU only"]
fn salon_end_cancels_its_exact_task_and_old_confirmation_cannot_cast_at_a_new_scope()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load(
            r#"
      local request=X2DialogManager.RequestTaskDialog
      X2DialogManager.RequestTaskDialog=function(self,task,info,owner)
        local w=request(self,task,info,owner)
        if task==DLG_TASK_CONFIRM_ENTER_BEAUTYSHOP then SALON_DIALOG=w end
        return w
      end
    "#,
        )
        .exec()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[22780], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: true
        }]
    );
    app.world().non_send::<UiRuntime>().lua.load("assert(SALON_DIALOG:IsVisible());OLD_SALON_OK=SALON_DIALOG.OkProc;X2Interaction:CancelNPCInteraction()").exec()?;
    app.update();
    let commands = drain(&mut app);
    assert_eq!(interactions(&commands), [Interact::NpcEnd { obj_id: A }]);
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, Command::UseSkill { .. }))
    );
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(not SALON_DIALOG:IsVisible())")
        .exec()?;
    query(&mut app, &shared, B);
    app.world_mut()
        .write_message(NetEvent(reply(B, &[22780], true)));
    app.update();
    drain(&mut app);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(SALON_DIALOG:IsVisible());OLD_SALON_OK(SALON_DIALOG)")
        .exec()?;
    app.update();
    assert!(
        !drain(&mut app)
            .iter()
            .any(|c| matches!(c, Command::UseSkill { .. })),
        "old originating token cannot become B"
    );
    hud::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "SALON_DIALOG.btnOk")?;
    app.update();
    assert!(
        drain(&mut app).iter().any(
            |c| matches!(c,Command::UseSkill{skill_id:22780,target_obj_id} if *target_obj_id==B)
        )
    );
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "ordinary registered callback without a produced window, original UI fixture"]
fn registered_callback_without_a_window_still_admits_after_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("SALON_CONFIRM=function() SALON_CALLS=SALON_CALLS+1 end")
        .exec()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[22780], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: true
        }]
    );
    assert!(active_scope(&shared.borrow()).is_some());
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(SALON_CALLS==1)")
        .exec()?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original store/auction windows and all service systems; CPU only"]
fn switching_one_offer_from_store_to_auction_does_not_let_old_cleanup_cancel_it()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12087, 12083], true)));
    app.update();
    drain(&mut app);
    let old = active_scope(&shared.borrow()).unwrap();
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(std::time::Duration::from_secs(1));
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Interaction:SelectInteraction(12083)")
        .exec()?;
    app.update(); // service-menu callback forwards the captured selection
    app.update(); // the ordinary owner dispatches it, then old store cleanup runs
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: false
        }]
    );
    let current = active_scope(&shared.borrow()).unwrap();
    assert_ne!(old, current);
    assert!(!scope_current(&shared.borrow(), old));
    assert!(scope_current(&shared.borrow(), current));
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(IsActivatedAuction() and not IsActivatedStore());assert(NPC_ENDS==1)")
        .exec()?;
    close_npc(&mut shared.borrow_mut(), old);
    app.update();
    assert!(interactions(&drain(&mut app)).is_empty());
    assert!(scope_current(&shared.borrow(), current));
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original CancelNPCInteraction binding and service callbacks; CPU only"]
fn callback_cancel_sends_early_interact_then_end_for_same_target()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12087], true)));
    app.update();
    drain(&mut app);
    app.world().non_send::<UiRuntime>().lua.load("SALON_CONFIRM=function() local before=NPC_ENDS;X2Interaction:CancelNPCInteraction();CANCEL_DELTA=NPC_ENDS-before end").exec()?;
    query(&mut app, &shared, B);
    app.world_mut()
        .write_message(NetEvent(reply(B, &[22780], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [
            Interact::NpcEnd { obj_id: A },
            Interact::Npc {
                obj_id: B,
                target_changed: true
            }
        ]
    );
    assert_eq!(active_scope(&shared.borrow()).unwrap().npc().0, B);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(CANCEL_DELTA==0 and NPC_ENDS==2)")
        .exec()?;
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(std::time::Duration::from_secs(1));
    query(&mut app, &shared, B);
    app.world_mut()
        .write_message(NetEvent(reply(B, &[22780], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [
            Interact::Npc {
                obj_id: B,
                target_changed: false
            },
            Interact::NpcEnd { obj_id: B }
        ],
        "the early 0x065 precedes the callback's end"
    );
    assert!(active_scope(&shared.borrow()).is_none());
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(CANCEL_DELTA==1 and NPC_ENDS==3)")
        .exec()?;
    query(&mut app, &shared, A);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Interaction:CancelNPCInteraction();assert(NPC_ENDS==3)")
        .exec()?;
    assert!(shared.borrow().dialog.npc_flow.pending.is_some());
    app.update();
    assert!(interactions(&drain(&mut app)).is_empty());
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original auction quote API and real command flusher; CPU only"]
fn anonymous_tooltip_quotes_survive_a_new_selected_auction_service()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Auction:GetLowestPrice(5569,1)")
        .exec()?;
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12083], true)));
    app.update();
    let commands = drain(&mut app);
    assert!(commands.iter().any(|c| matches!(c,Command::Auction(aa_net::AuctionCommand::LowestPrice{auctioneer,item_template_id:5569,..}) if auctioneer.obj_id==0 && auctioneer.obj_id2==0)));
    assert_eq!(
        interactions(&commands),
        [Interact::Npc {
            obj_id: A,
            target_changed: true
        }]
    );
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "actual HUD synchronization and recruiter callback on original UI; CPU only"]
fn recruiter_receipt_uses_the_target_copied_by_the_preceding_hud_sync()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, B);
    assert_eq!(shared.borrow().panels.target.as_ref().unwrap().obj_id, A);
    let spawn = app.world().resource::<Player>().spawn.clone().unwrap();
    let mut target = aa_net::state::Target::default();
    target.apply(&Event::PlayerSpawn(Box::new(spawn.clone())));
    let changed = Event::TargetChanged {
        obj_id: spawn.obj_id,
        target_obj_id: B,
    };
    assert!(target.apply(&changed));
    app.insert_resource(target).add_systems(
        Update,
        hud::sync_hud
            .after(track_npc)
            .before(super::super::handle_dialog),
    );
    app.world_mut().write_message(NetEvent(changed));
    app.world_mut()
        .write_message(NetEvent(reply(B, &[12660], true)));
    app.update();
    assert_eq!(shared.borrow().panels.target.as_ref().unwrap().obj_id, B);
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: B,
            target_changed: true
        }]
    );
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(NPC_KINDS[#NPC_KINDS]=='recruiter')")
        .exec()?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "actual query intent owner and original service callback; CPU only"]
fn later_explicit_intent_retries_the_same_unanswered_npc_but_same_pass_duplicates_coalesce()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    let old = shared
        .borrow()
        .dialog
        .npc_flow
        .pending
        .as_ref()
        .unwrap()
        .scope;
    query(&mut app, &shared, A);
    assert_ne!(
        shared
            .borrow()
            .dialog
            .npc_flow
            .pending
            .as_ref()
            .unwrap()
            .scope,
        old
    );
    query_npc(&mut shared.borrow_mut(), (A, TEMPLATE), None);
    query_npc(&mut shared.borrow_mut(), (A, TEMPLATE), None);
    assert_eq!(shared.borrow().dialog.npc_flow.intents.len(), 1);
    app.update();
    assert_eq!(interactions(&drain(&mut app)).len(), 1);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12087], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: true
        }]
    );
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(IsActivatedStore() and #NPC_KINDS==1)")
        .exec()?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original END listeners and actual post-query range rejection; CPU only"]
fn changed_target_cancels_prior_activity_before_deadline_write_and_later_range_rejection()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12087], true)));
    app.update();
    drain(&mut app);
    let old_deadline = shared.borrow().dialog.npc_flow.next_action_ms;
    {
        let lua = &app.world().non_send::<UiRuntime>().lua;
        let b = shared.clone();
        lua.globals().set(
            "deadlineProbe",
            lua.create_function(move |_, ()| Ok(b.borrow().dialog.npc_flow.next_action_ms))?,
        )?;
        lua.load("DEADLINE_OWNER=UIParent:CreateWidget('emptywidget','npcDeadlineProbe','UIParent');DEADLINE_OWNER:RegisterEvent('NPC_INTERACTION_END');DEADLINE_OWNER:SetHandler('OnEvent',function(self,event) if event=='NPC_INTERACTION_END' then AT_END_DEADLINE=deadlineProbe() end end)").exec()?;
    }
    query(&mut app, &shared, B);
    app.world_mut()
        .resource_mut::<Units>()
        .by_id
        .get_mut(&B)
        .unwrap()
        .pos = aa_net::Pos::new(100., 0., 0.);
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(std::time::Duration::from_secs(1));
    app.world_mut()
        .write_message(NetEvent(reply(B, &[12083], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::NpcEnd { obj_id: A }]
    );
    assert!(active_scope(&shared.borrow()).is_none());
    assert_eq!(
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .globals()
            .get::<u32>("AT_END_DEADLINE")?,
        old_deadline
    );
    let now = app
        .world()
        .resource::<Time<Virtual>>()
        .elapsed()
        .as_millis() as u32;
    assert_eq!(
        shared.borrow().dialog.npc_flow.next_action_ms,
        now.wrapping_add(500)
    );
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(NPC_ENDS==2 and not IsActivatedStore() and not IsActivatedAuction())")
        .exec()?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "actual screen reset and retained original UI without a synthetic respawn; CPU only"]
fn loading_reset_retires_old_tokens_without_losing_received_identity_or_reusing_stamps()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    app.add_systems(OnExit(crate::Screen::InGame), super::super::reset);
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12087], true)));
    app.update();
    drain(&mut app);
    let old = active_scope(&shared.borrow()).unwrap();
    let deadline = shared.borrow().dialog.npc_flow.next_action_ms;
    app.world_mut()
        .resource_mut::<NextState<crate::Screen>>()
        .set(crate::Screen::Loading);
    app.update();
    assert!(active_scope(&shared.borrow()).is_none());
    {
        let data = shared.borrow();
        assert_eq!(data.dialog.npc_flow.actor, Some((old.character, old.actor)));
        assert_eq!(data.dialog.npc_flow.next_action_ms, deadline);
        assert!(data.dialog.npc_flow.epoch > old.epoch);
        assert!(!scope_current(&data, old));
    }
    app.world_mut()
        .resource_mut::<NextState<crate::Screen>>()
        .set(crate::Screen::InGame);
    app.update();
    drain(&mut app);
    // No PlayerSpawn or NPC announcement is injected on this UI-only transition.
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12087], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: true
        }]
    );
    let current = active_scope(&shared.borrow()).unwrap();
    assert_ne!(old, current);
    close_npc(&mut shared.borrow_mut(), old);
    assert!(scope_current(&shared.borrow(), current));
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original UI/catalog and all six service plugins, CPU only; no network/GPU"]
fn data_only_offer_sends_the_1_2_early_and_tail_interact_packets()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[25205], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [
            Interact::Npc {
                obj_id: A,
                target_changed: true
            },
            Interact::Npc {
                obj_id: A,
                target_changed: false
            }
        ],
        "early 0x065 announces the talk; the tail reports no further change"
    );
    assert_eq!(shared.borrow().dialog.npc, Some((A, TEMPLATE)));
    assert!(active_scope(&shared.borrow()).is_some());
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original UI/catalog and all six service plugins, CPU only; no network/GPU"]
fn repeat_data_only_talk_reports_target_unchanged() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[25205], true)));
    app.update();
    drain(&mut app);
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(std::time::Duration::from_secs(1));
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[25205], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [
            Interact::Npc {
                obj_id: A,
                target_changed: false
            },
            Interact::Npc {
                obj_id: A,
                target_changed: false
            }
        ]
    );
    assert_eq!(shared.borrow().dialog.npc, Some((A, TEMPLATE)));
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original UI/catalog and all six service plugins, CPU only; no network/GPU"]
fn switching_after_a_data_only_trader_talk_ends_it_before_the_new_talk()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[25205], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [
            Interact::Npc {
                obj_id: A,
                target_changed: true
            },
            Interact::Npc {
                obj_id: A,
                target_changed: false
            }
        ]
    );
    assert_eq!(shared.borrow().dialog.npc, Some((A, TEMPLATE)));
    query(&mut app, &shared, B);
    app.world_mut()
        .write_message(NetEvent(reply(B, &[12087], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [
            Interact::NpcEnd { obj_id: A },
            Interact::Npc {
                obj_id: B,
                target_changed: true
            }
        ],
        "the trader's end precedes the new NPC's early 0x065"
    );
    assert_eq!(shared.borrow().dialog.npc, Some((B, TEMPLATE)));
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original UI/catalog and all six service plugins, CPU only; no network/GPU"]
fn unresolved_data_only_offer_sends_no_interact_packet() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut().resource_mut::<Units>().by_id.remove(&A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[25205], true)));
    app.update();
    assert!(interactions(&drain(&mut app)).is_empty());
    assert_eq!(shared.borrow().dialog.npc, None);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original salon callback and all six service plugins, CPU only"]
fn cancelled_talk_clears_dialog_npc_like_upstream() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[22780], true)));
    app.update();
    drain(&mut app);
    assert_eq!(shared.borrow().dialog.npc, Some((A, TEMPLATE)));
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("SALON_CONFIRM=function() X2Interaction:CancelNPCInteraction() end")
        .exec()?;
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(std::time::Duration::from_secs(1));
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[22780], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [
            Interact::Npc {
                obj_id: A,
                target_changed: false
            },
            Interact::NpcEnd { obj_id: A }
        ],
        "the early 0x065 precedes the callback's end"
    );
    assert_eq!(
        shared.borrow().dialog.npc,
        None,
        "a cancelled local offer keeps the upstream dialog.npc clearing"
    );
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original UI/catalog and all six service plugins, CPU only; no network/GPU"]
fn window_talk_sends_exactly_one_interact_packet_per_talk() -> Result<(), Box<dyn std::error::Error>>
{
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12087], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: true
        }],
        "the 1.2 merchant capture (store window opened) has one 0x065 per talk; no tail"
    );
    assert_eq!(shared.borrow().dialog.npc, Some((A, TEMPLATE)));
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(std::time::Duration::from_secs(1));
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12087], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: false
        }],
        "repeat talk: the early 0x065 reports no target change"
    );
    assert_eq!(shared.borrow().dialog.npc, Some((A, TEMPLATE)));
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "original UI/catalog and all six service plugins, CPU only; no network/GPU"]
fn ordinary_talk_without_a_local_window_still_sends_the_early_interact()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[99999], true)));
    app.update();
    assert_eq!(
        interactions(&drain(&mut app)),
        [Interact::Npc {
            obj_id: A,
            target_changed: true
        }],
        "the captured ordinary NPC opens no local window and still sends its 0x065"
    );
    assert_eq!(shared.borrow().dialog.npc, None);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
#[ignore = "actual ordered replacement receipts and outbound interaction queue; CPU only"]
fn replacement_retires_queued_end_and_pointer_intent_without_affecting_another_npc()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut app, shared, path) = harness()?;
    query(&mut app, &shared, A);
    app.world_mut()
        .write_message(NetEvent(reply(A, &[12087], true)));
    app.update();
    drain(&mut app);
    query(&mut app, &shared, B); // B pending, so the copied A offer is already gone.
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Interaction:CancelNPCInteraction()")
        .exec()?;
    assert_eq!(shared.borrow().dialog.npc_flow.ends.len(), 1);
    let replacement = app.world().resource::<Units>().get(A).unwrap().clone();
    app.world_mut()
        .write_message(NetEvent(Event::UnitSpawned(Box::new(replacement.clone()))));
    app.update();
    assert!(
        interactions(&drain(&mut app)).is_empty(),
        "old066 must not target a replacement with identical IDs"
    );
    assert!(
        shared.borrow().dialog.npc_flow.pending.is_some(),
        "unrelated B discovery survives"
    );
    app.world_mut()
        .write_message(NetEvent(reply(B, &[12087], true)));
    app.update();
    drain(&mut app);
    let b = active_scope(&shared.borrow()).unwrap();
    query_npc(&mut shared.borrow_mut(), (A, TEMPLATE), None);
    app.world_mut()
        .write_message(NetEvent(Event::UnitSpawned(Box::new(replacement))));
    app.update();
    assert!(
        interactions(&drain(&mut app)).is_empty(),
        "old pointer intent must not query a replacement"
    );
    assert!(scope_current(&shared.borrow(), b));
    query(&mut app, &shared, A); // A fresh explicit intent remains available.
    std::fs::remove_file(path)?;
    Ok(())
}
