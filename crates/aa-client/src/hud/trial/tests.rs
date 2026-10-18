//! Original stage and received-context checks for trial APIs and task controls.
use super::*;
use aa_net::{TrialEvent as E, TrialRecords};
use std::{cell::RefCell, rc::Rc};

fn setup() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", 1920., 1080.)?;
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let state = Rc::new(RefCell::new(HudData::default()));
    super::super::api::install(&mut rt, &state)?;
    fixture::selected(Some(&rt), &state);
    Ok((rt, state))
}
fn commands(state: &Shared) -> Vec<(u32, TrialCommand)> {
    flush(&mut state.borrow_mut())
        .into_iter()
        .map(|command| match command {
            aa_net::Command::Trial { character_id, command } => (character_id, command),
            other => panic!("unexpected command {other:?}"),
        })
        .collect()
}
fn send(rt: &UiRuntime, state: &Shared, event: E) {
    fixture::send(Some(rt), state, event);
}

#[test]
fn callbacks_replay_native_pre_adoption_snapshots_before_final_received_state() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua
        .load(
            r#"
        observed={}
        local w=UIParent:CreateWidget('emptywidget','trialListener','UIParent')
        w:SetHandler('OnEvent',function(self,event,...)
          local data=X2Trial:GetCrimeData()
          observed[#observed+1]={event=event,args={...},jury=data.isJury,total=data.total,kind=X2Trial:GetTrialType()}
        end)
        w:RegisterEvent('TRIAL_STATUS');w:RegisterEvent('UPDATE_INGAME_BEAUTYSHOP_STATUS')
    "#,
        )
        .exec()?;
    fixture::send(
        None,
        &state,
        E::PhaseChanged {
            trial_id: 7,
            phase: 1,
            current_jury: -1,
            remaining_time_ms: 60_000,
        },
    );
    fixture::send(
        None,
        &state,
        E::SummonJury {
            trial_id: 8,
            court: 4,
            jury_number: 2,
        },
    );
    rt.lua.load("assert(#observed==0 and X2Trial:GetCrimeData().isJury)").exec()?;
    replay(&rt, &state);
    rt.lua
        .load(
            r#"
        assert(#observed==3)
        assert(observed[1].event=='TRIAL_STATUS' and observed[1].args[1]==1 and observed[1].args[2]==-1)
        assert(observed[1].total==nil and observed[1].kind==nil and not observed[1].jury)
        assert(observed[2].event=='UPDATE_INGAME_BEAUTYSHOP_STATUS' and observed[2].total==0 and not observed[2].jury)
        assert(observed[3].event=='UPDATE_INGAME_BEAUTYSHOP_STATUS' and not observed[3].jury)
        assert(X2Trial:GetCrimeData().isJury and X2Trial:GetCrimeData().total==0)
    "#,
        )
        .exec()?;
    assert!(state.borrow().trial.callback.is_none());
    assert!(state.borrow().trial.pending.is_empty());
    Ok(())
}

#[test]
fn received_getters_preserve_19_native_keys_kind_snapshot_signed_fields_and_page_layout() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    fixture::records(Some(&rt), &state);
    let mut history = fixture::history();
    history.crime_point = i32::MAX;
    history.total_seconds = i32::MIN;
    history.crime_record_count = 17;
    history.bot_report = 29;
    send(&rt, &state, E::History(history));
    // Appending rows does not recompute the native cached kind counts until176.
    for first in [6, 11, 16] {
        let count = if first == 16 { 2 } else { 5 };
        send(
            &rt,
            &state,
            E::Records(TrialRecords {
                trial_id: 999,
                raw_unknown: 0,
                total_count: 17,
                count,
                records: (first..first + count as u32).map(|id| fixture::row(id, 4)).collect(),
            }),
        );
    }
    rt.lua
        .load(
            r#"
        local d=X2Trial:GetCrimeData();local n=0;for _ in pairs(d) do n=n+1 end
        assert(n==19 and d.isJury and d.defendant=='Received defendant' and d.race=='Nuian')
        assert(d.murder==2 and d.assault==1 and d.theft==1 and d.etc==1 and d.total==17)
        assert(d.crimePoint==2147483648 and d.totalSeconds==-2147483648 and d.totalRecords==17)
        assert(#X2Trial:GetCrimeRecordsByPage(0)==0 and #X2Trial:GetCrimeRecordsByPage(-1)==0)
        local p=X2Trial:GetCrimeRecordsByPage(1);assert(#p==10 and #p[1]==10)
        assert(p[1][1]=='1' and p[1][2]=='Reporter' and p[1][3]==3 and p[1][4]=='Witness')
        assert(p[1][5]=='A received crime record' and p[1][7]=='' and p[1][8]=='' and p[1][9]=='')
        assert(p[1][10]~='' and #X2Trial:GetCrimeRecordsByPage(2)==7 and #X2Trial:GetCrimeRecordsByPage(3)==0)
        assert(select('#',X2Trial:GetCrimeRecordsByPage('1'))==0)
    "#,
        )
        .exec()?;
    send(
        &rt,
        &state,
        E::Records(TrialRecords {
            trial_id: 999,
            raw_unknown: u32::MAX,
            total_count: -1,
            count: 0,
            records: vec![],
        }),
    );
    let mut defendant = state.borrow().trial.received.defendant.clone().unwrap();
    defendant.race = 255;
    defendant.faction_id = 99;
    defendant.sentence_time_ms = -1;
    send(&rt, &state, E::DefendantData(defendant));
    rt.lua
        .load(
            r#"
        local d=X2Trial:GetCrimeData()
        assert(X2Trial:GetTrialType()==-1 and d.totalRecords==29 and d.total==-1)
        assert(d.race=='invalid char_race' and d.faction=='' and d.defaultSentenceTime==4294967296)
        assert(d.theft==13 and d.murder==2)
    "#,
        )
        .exec()?;
    Ok(())
}

#[test]
fn native_weak_controls_return_no_values_and_cannot_cross_received_context_or_session() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua
        .load("assert(select('#',X2Trial:ConfirmCrimeRecords())==0);X2Trial:CancelTrial();X2Trial:ChooseVerdict(1)")
        .exec()?;
    assert!(commands(&state).is_empty());
    send(
        &rt,
        &state,
        E::SummonJury {
            trial_id: 7,
            court: 4,
            jury_number: -9,
        },
    );
    send(
        &rt,
        &state,
        E::JuryWaiting {
            count: 0,
            total: 5,
            sentence_time_ms: 0,
        },
    );
    rt.lua
        .load(
            r#"
        assert(select('#',X2Trial:ConfirmCrimeRecords())==0)
        assert(select('#',X2Trial:CancelTrial())==0)
        assert(select('#',X2Trial:ChooseVerdict(6.9))==0)
        X2Trial:ChooseVerdict('1');X2Trial:ChooseVerdict(0/0);X2Trial:ChooseVerdict(7)
    "#,
        )
        .exec()?;
    assert_eq!(
        commands(&state),
        [
            (
                47,
                TrialCommand::ConfirmCrimeRecords {
                    trial_id: 7,
                    jury_number: -9
                }
            ),
            (47, TrialCommand::CancelTrial { trial_id: 7 }),
            (
                47,
                TrialCommand::ChooseVerdict {
                    trial_id: 7,
                    jury_number: -9,
                    choice: 6
                }
            )
        ]
    );
    // Reset retains the native seat; verdict has no invented jury/phase/nonzero-ID gate.
    send(&rt, &state, E::Canceled { trial_id: 999 });
    rt.lua.load("X2Trial:ChooseVerdict(1);X2Trial:ConfirmCrimeRecords()").exec()?;
    assert_eq!(
        commands(&state),
        [(
            47,
            TrialCommand::ChooseVerdict {
                trial_id: 0,
                jury_number: -9,
                choice: 1
            }
        )]
    );
    rt.lua.load("X2Trial:ChooseVerdict(2)").exec()?;
    dispatch_at(None, &state, &Event::Disconnected { message: "fixture".into() }, None);
    fixture::selected(None, &state);
    assert!(
        commands(&state).is_empty(),
        "same character ID must not inherit requests from its previous session"
    );
    assert!(!state.borrow().trial.received.is_jury);
    Ok(())
}

#[test]
fn original_task_controls_retain_captured_ids_duplicate_rules_and_deferred_visual_admission() -> Result<(), Box<dyn std::error::Error>> {
    let (mut rt, state) = setup()?;
    let path = crate::settings::isolated_bindings(&rt)?;
    rt.lua
        .load(
            r#"
      TASK_FIXTURE={};TASK_INFO={}
      local request=X2DialogManager.RequestTaskDialog
      function X2DialogManager:RequestTaskDialog(task,info,owner)
        local w=request(self,task,info,owner);TASK_FIXTURE[task]=w;TASK_INFO[task]=info;return w
      end
      X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_JURY_INVITATION,0)
    "#,
        )
        .exec()?;
    send(
        &rt,
        &state,
        E::JuryInvited {
            defendant_name: b"First\0unused".to_vec(),
            trial_id: 700,
        },
    );
    rt.lua.load("assert(TASK_FIXTURE[6]==nil)").exec()?;
    assert!(commands(&state).is_empty());
    // Generic visual failure retains the task; retries do not re-run CanStart.
    rt.lua
        .load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_JURY_INVITATION,1)")
        .exec()?;
    // The original skill window waits for the real static database. This minimal
    // API fixture has no background loader, so supply it before building the HUD.
    let game = super::super::data::GameData::load(aa_anim::db::DEFAULT_DB)?;
    assert!(state.borrow().game.0.set(game).is_ok());
    rt.run_stage(aa_ui::Stage::World);
    tasks::show(&rt.lua, &state);
    aa_ui::lua::flush(&rt.lua);
    rt.lua
        .load("assert(TASK_FIXTURE[6]:IsVisible() and TASK_INFO[6].defendantName=='First' and TASK_INFO[6].isModal==false)")
        .exec()?;
    super::super::expansion::fixture::click(&rt, "TASK_FIXTURE[6].btnOk")?;
    assert_eq!(
        commands(&state),
        [(
            47,
            TrialCommand::ReplyJuryInvitation {
                accept: true,
                trial_id: 700
            }
        )]
    );
    send(
        &rt,
        &state,
        E::JuryInvited {
            defendant_name: b"Ignored".to_vec(),
            trial_id: 900,
        },
    );
    assert_eq!(
        commands(&state),
        [(
            47,
            TrialCommand::ReplyJuryInvitation {
                accept: false,
                trial_id: 900
            }
        )]
    );
    rt.lua
        .load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_JURY_INVITATION,0)")
        .exec()?;
    send(
        &rt,
        &state,
        E::JuryInvited {
            defendant_name: b"Old quote".to_vec(),
            trial_id: 701,
        },
    );
    send(
        &rt,
        &state,
        E::JuryInvited {
            defendant_name: b"New quote".to_vec(),
            trial_id: 801,
        },
    );
    assert_eq!(
        commands(&state),
        [(
            47,
            TrialCommand::ReplyJuryInvitation {
                accept: false,
                trial_id: 801
            }
        )]
    );
    state.borrow_mut().trial.main_world = Some(false);
    super::super::expansion::fixture::click(&rt, "TASK_FIXTURE[6].btnOk")?;
    assert_eq!(
        commands(&state),
        [(
            47,
            TrialCommand::ReplyJuryInvitation {
                accept: false,
                trial_id: 701
            }
        )]
    );
    send(
        &rt,
        &state,
        E::ImprisonOrTrialOffered {
            crime_value: 60,
            jail_minutes: 30,
        },
    );
    send(
        &rt,
        &state,
        E::ImprisonOrTrialOffered {
            crime_value: 99,
            jail_minutes: 50,
        },
    );
    rt.lua
        .load("assert(TASK_INFO[7].crimeValue==60 and TASK_INFO[7].jailMinutes==30 and TASK_INFO[7].isModal)")
        .exec()?;
    assert!(commands(&state).is_empty());
    super::super::expansion::fixture::click(&rt, "TASK_FIXTURE[7].btnCancel")?;
    assert_eq!(
        commands(&state),
        [(47, TrialCommand::ReplyImprisonOrTrial { request_trial: false })]
    );
    send(&rt, &state, E::SummonDefendant { trial_id: 1000 });
    send(
        &rt,
        &state,
        E::PhaseChanged {
            trial_id: 9999,
            phase: 4,
            current_jury: 5,
            remaining_time_ms: 1000,
        },
    );
    send(
        &rt,
        &state,
        E::PhaseChanged {
            trial_id: 8888,
            phase: 4,
            current_jury: 5,
            remaining_time_ms: 1000,
        },
    );
    super::super::expansion::fixture::click(&rt, "TASK_FIXTURE[8].btnOk")?;
    assert_eq!(commands(&state), [(47, TrialCommand::SkipFinalStatement { trial_id: 9999 })]);
    send(
        &rt,
        &state,
        E::PhaseChanged {
            trial_id: 7777,
            phase: 4,
            current_jury: 5,
            remaining_time_ms: 1000,
        },
    );
    super::super::expansion::fixture::click(&rt, "TASK_FIXTURE[8].btnCancel")?;
    assert!(commands(&state).is_empty());
    send(
        &rt,
        &state,
        E::PhaseChanged {
            trial_id: 6666,
            phase: 4,
            current_jury: 5,
            remaining_time_ms: 1000,
        },
    );
    send(
        &rt,
        &state,
        E::PhaseChanged {
            trial_id: 5555,
            phase: 5,
            current_jury: 5,
            remaining_time_ms: 1000,
        },
    );
    rt.lua.load("assert(not TASK_FIXTURE[8]:IsVisible())").exec()?;
    assert!(commands(&state).is_empty(), "phase5 closes Task8 with its no-wire false callback");
    send(
        &rt,
        &state,
        E::JuryInvited {
            defendant_name: b"Pending".to_vec(),
            trial_id: 702,
        },
    );
    send(
        &rt,
        &state,
        E::ImprisonOrTrialOffered {
            crime_value: 61,
            jail_minutes: 31,
        },
    );
    send(&rt, &state, E::Canceled { trial_id: 999 });
    assert_eq!(
        commands(&state),
        [
            (
                47,
                TrialCommand::ReplyJuryInvitation {
                    accept: false,
                    trial_id: 702
                }
            ),
            (47, TrialCommand::ReplyImprisonOrTrial { request_trial: false })
        ]
    );
    rt.lua.load("HUD.trial_close_tasks()").exec()?;
    assert!(commands(&state).is_empty(), "retired task dialogs cannot send a second answer");
    send(
        &rt,
        &state,
        E::JuryInvited {
            defendant_name: b"Old connection".to_vec(),
            trial_id: 703,
        },
    );
    send(
        &rt,
        &state,
        E::ImprisonOrTrialOffered {
            crime_value: 62,
            jail_minutes: 32,
        },
    );
    dispatch_at(Some(&rt), &state, &Event::Disconnected { message: "fixture".into() }, None);
    fixture::selected(Some(&rt), &state);
    assert!(
        commands(&state).is_empty(),
        "same-ID re-entry retires old captures before closing their widgets"
    );
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn original_records_verdict_and_waiting_buttons_send_captured_commands_without_optimism() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::breath::fixture::runtime(1920., 1080.)?;
    fixture::selected(Some(&rt), &state);
    rt.lua.load("ADDON:ShowContent(UIC_BAG,false)").exec()?;
    fixture::records(Some(&rt), &state);
    aa_ui::lua::flush(&rt.lua);
    let timer = state.borrow_mut().trial.received.advance_time(1_700_000_001).unwrap();
    emit(Some(&rt), &state, timer)?;
    rt.lua
        .load(
            r#"
        local period={year=0,month=0,day=0,hour=0,minute=0,second=59}
        local expected=locale.trial.remainTime(locale.time.GetPeriodToMinutesSecondFormat(period))
        assert(userTrial.crimeRecords.stepInfoFrame.remainTime:GetText()==expected)
        X2Trial:GetCrimeData();X2Trial:GetCrimeRecordsByPage(1)
    "#,
        )
        .exec()?;
    assert_eq!(state.borrow().trial.received.last_epoch_seconds, Some(1_700_000_001));
    assert!(state.borrow_mut().trial.received.advance_time(1_700_000_001).is_none());
    rt.lua
        .load("assert(userTrial.crimeRecords:IsVisible() and userTrial.crimeRecords.judgmentButton:IsEnabled())")
        .exec()?;
    super::super::expansion::fixture::click(&rt, "userTrial.crimeRecords.judgmentButton")?;
    assert_eq!(
        commands(&state),
        [(
            47,
            TrialCommand::ConfirmCrimeRecords {
                trial_id: 7,
                jury_number: 2
            }
        )]
    );
    assert_eq!(state.borrow().trial.received.jury_ok_count, Some(0));
    send(
        &rt,
        &state,
        E::PhaseChanged {
            trial_id: 7,
            phase: 5,
            current_jury: 5,
            remaining_time_ms: 60_000,
        },
    );
    aa_ui::lua::flush(&rt.lua);
    rt.lua
        .load(
            r#"
        assert(userTrial.verdictWnd:IsVisible())
        for i,v in ipairs({6,15,24,30,36}) do assert(userTrial.verdictWnd.verdictBtn[i+1].amount==v) end
    "#,
        )
        .exec()?;
    super::super::expansion::fixture::click(&rt, "userTrial.verdictWnd.verdictBtn[4]")?;
    super::super::expansion::fixture::click(&rt, "userTrial.verdictWnd.okButton")?;
    assert_eq!(
        commands(&state),
        [(
            47,
            TrialCommand::ChooseVerdict {
                trial_id: 7,
                jury_number: 2,
                choice: 4
            }
        )]
    );
    assert_eq!(state.borrow().trial.received.verdict_counts, Some([0, 0]));
    send(&rt, &state, E::Canceled { trial_id: 999 });
    send(&rt, &state, E::SummonDefendant { trial_id: 9 });
    send(
        &rt,
        &state,
        E::Waiting {
            order: 17,
            sentence_time_ms: 1_800_000,
        },
    );
    send(
        &rt,
        &state,
        E::JuryWaiting {
            count: 1,
            total: 5,
            sentence_time_ms: 1_800_000,
        },
    );
    rt.lua
        .load("assert(userTrial.defendantWaitWnd:IsVisible() and not userTrial.defendantWaitWnd.recognitionBtn:IsEnabled())")
        .exec()?;
    send(
        &rt,
        &state,
        E::JuryWaiting {
            count: 0,
            total: 5,
            sentence_time_ms: 1_800_000,
        },
    );
    aa_ui::lua::flush(&rt.lua);
    super::super::expansion::fixture::click(&rt, "userTrial.defendantWaitWnd.recognitionBtn")?;
    assert_eq!(commands(&state), [(47, TrialCommand::CancelTrial { trial_id: 9 })]);
    assert!(state.borrow().trial.received.is_defendant());
    assert_eq!(state.borrow().trial.received.trial_id, 9);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn hud_schedule_uses_actual_layer_and_login_start_and_writes_real_typed_commands() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
        .init_state::<crate::Screen>()
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(super::super::HudBuilt { up: true, fresh: false })
        .insert_resource(crate::login::NativeLoginStarted(true))
        .insert_resource(aa_world::WorldConfig::new("main_world"))
        .insert_resource(aa_world::stream::WorldInfo(std::sync::Arc::new(aa_world::World::parse_g(
            "name main_world\n",
        ))))
        .add_message::<NetEvent>()
        .add_message::<NetCommand>();
    plugin(&mut app);
    app.world_mut().write_message(NetEvent(Event::Trial(E::SummonJury {
        trial_id: 7,
        court: 4,
        jury_number: 2,
    })));
    app.update();
    assert!(state.borrow().trial.login_started);
    assert_eq!(
        state.borrow().trial.main_world,
        None,
        "preloaded world metadata is not the login layer"
    );
    app.world_mut()
        .resource_mut::<NextState<crate::Screen>>()
        .set(crate::Screen::InGame);
    app.update();
    assert_eq!(state.borrow().trial.main_world, Some(true));
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Trial:ConfirmCrimeRecords()")
        .exec()?;
    app.update();
    let sent = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|message| message.0)
        .collect::<Vec<_>>();
    assert!(matches!(
        sent.as_slice(),
        [aa_net::Command::Trial {
            character_id: 47,
            command: TrialCommand::ConfirmCrimeRecords {
                trial_id: 7,
                jury_number: 2
            }
        }]
    ));
    app.world_mut().resource_mut::<aa_world::WorldConfig>().world = "login2".into();
    app.update();
    assert_eq!(
        state.borrow().trial.main_world,
        None,
        "a stale loaded layer cannot admit the invitation"
    );
    Ok(())
}
