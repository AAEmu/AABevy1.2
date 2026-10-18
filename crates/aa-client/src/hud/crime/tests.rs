use super::*;
use aa_net::CrimeEvent as Receipt;
use std::{cell::RefCell, rc::Rc};

fn setup() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", 1280., 720.)?;
    let state = Rc::new(RefCell::new(super::super::api::HudData::default()));
    super::super::api::install(&mut rt, &state)?;
    rt.lua
        .load(
            r#"
        observed={}
        local listener=UIParent:CreateWidget('emptywidget','crimeListener','UIParent')
        listener:SetHandler('OnEvent',function(self,event,...)
            local info=X2Player:GetCrimeInfo()
            table.insert(observed,{event=event,args={...},info=info,jury=X2Player:GetJuryPoint()})
        end)
        listener:RegisterEvent('CRIME_REPORTED');listener:RegisterEvent('PLAYER_JURY_POINT');listener:RegisterEvent('JURY_WAITING_NUMBER')
    "#,
        )
        .exec()?;
    Ok((rt, state))
}
#[test]
fn selected_header_getters_and_callbacks_preserve_native_rounding_wrapping_and_duplicates() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    let mut header = fixture::header();
    header.character.crime_point = i16::MAX;
    header.character.infamy_point = i32::MIN;
    header.character.crime_state = i16::MIN;
    dispatch(Some(&rt), &state, &Event::CharacterSelected(Box::new(header)));
    rt.lua
        .load(
            r#"
        local info=X2Player:GetCrimeInfo();local count=0;for _ in pairs(info) do count=count+1 end
        assert(count==3 and info.crimePoint==32767 and info.crimeRecord==-2147483648 and info.crimeScore==-32768)
        assert(X2Player:GetJuryPoint()==8 and #observed==0)
    "#,
        )
        .exec()?;
    let changed = Event::Crime(Receipt::Changed {
        points: 7,
        crime_points: i16::MIN,
        infamy_points: i32::MAX,
        crime_state: i16::MAX,
    });
    dispatch(Some(&rt), &state, &changed);
    dispatch(Some(&rt), &state, &changed);
    for _ in 0..2 {
        dispatch(Some(&rt), &state, &Event::Crime(Receipt::JuryPointChanged { jury_point: i32::MAX }));
    }
    dispatch(
        Some(&rt),
        &state,
        &Event::Crime(Receipt::JuryWaitingNumber { waiting_number: i32::MIN }),
    );
    rt.lua
        .load(
            r#"
        assert(#observed==5)
        local a=observed[1];assert(a.event=='CRIME_REPORTED' and #a.args==3)
        assert(a.args[1]==1 and a.args[2]==-1 and a.args[3]==-1)
        assert(a.info.crimePoint==-32768 and a.info.crimeRecord==2147483648 and a.info.crimeScore==32767)
        a=observed[2];assert(a.args[1]==0 and a.args[2]==0 and a.args[3]==0)
        for i=3,4 do assert(observed[i].event=='PLAYER_JURY_POINT' and #observed[i].args==0 and observed[i].jury==2147483648) end
        assert(observed[5].event=='JURY_WAITING_NUMBER' and observed[5].args[1]==-2147483648)
    "#,
        )
        .exec()?;
    Ok(())
}
#[test]
fn unknown_header_stays_unknown_and_resets_do_not_invent_crime_callbacks() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua
        .load("assert(next(X2Player:GetCrimeInfo())==nil and X2Player:GetJuryPoint()==0)")
        .exec()?;
    dispatch(
        Some(&rt),
        &state,
        &Event::Crime(Receipt::Changed {
            points: 1,
            crime_points: 60,
            infamy_points: 123,
            crime_state: 2,
        }),
    );
    dispatch(Some(&rt), &state, &Event::Crime(Receipt::JuryPointChanged { jury_point: 5 }));
    rt.lua.load("assert(#observed==0 and next(X2Player:GetCrimeInfo())==nil)").exec()?;
    dispatch(Some(&rt), &state, &Event::CharacterSelected(Box::new(fixture::header())));
    dispatch(
        Some(&rt),
        &state,
        &Event::Disconnected {
            message: "fixture disconnect".into(),
        },
    );
    rt.lua.load("assert(#observed==0 and next(X2Player:GetCrimeInfo())==nil)").exec()?;
    // Queue-number dispatch is manager-owned and does not manufacture a selected actor.
    dispatch(Some(&rt), &state, &Event::Crime(Receipt::JuryWaitingNumber { waiting_number: -1 }));
    rt.lua
        .load("assert(#observed==1 and observed[1].args[1]==-1 and next(observed[1].info)==nil)")
        .exec()?;
    assert!(state.borrow().crime.received.character_id.is_none());
    Ok(())
}
#[test]
fn jury_hover_requests_capture_identity_return_no_values_and_never_change_received_state() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua.load("assert(select('#',X2Trial:RequestJuryWaitingNumber())==0)").exec()?;
    assert!(flush(&mut state.borrow_mut()).is_empty());
    state.borrow_mut().character_id = 47;
    dispatch(None, &state, &Event::CharacterSelected(Box::new(fixture::header())));
    rt.lua
        .load("assert(select('#',X2Trial:RequestJuryWaitingNumber())==0);X2Trial:RequestJuryWaitingNumber()")
        .exec()?;
    assert!(matches!(
        flush(&mut state.borrow_mut()).as_slice(),
        [
            aa_net::Command::RequestJuryWaitingNumber { character_id: 47 },
            aa_net::Command::RequestJuryWaitingNumber { character_id: 47 }
        ]
    ));
    assert_eq!(state.borrow().crime.received.jury_point, Some(8));
    assert!(state.borrow().crime.received.last_receipt.is_none());
    rt.lua.load("X2Trial:RequestJuryWaitingNumber()").exec()?;
    state.borrow_mut().character_id = 48;
    assert!(flush(&mut state.borrow_mut()).is_empty());
    state.borrow_mut().character_id = 47;
    rt.lua.load("X2Trial:RequestJuryWaitingNumber()").exec()?;
    dispatch(
        None,
        &state,
        &Event::Disconnected {
            message: "fixture disconnect".into(),
        },
    );
    dispatch(None, &state, &Event::CharacterSelected(Box::new(fixture::header())));
    assert!(
        flush(&mut state.borrow_mut()).is_empty(),
        "same ID in a new connection must not inherit the old request"
    );
    rt.lua.load("X2Trial:RequestJuryWaitingNumber()").exec()?;
    state.borrow_mut().disconnected = true;
    assert!(flush(&mut state.borrow_mut()).is_empty());
    Ok(())
}
#[test]
fn receipt_dispatch_and_real_command_writer_run_in_the_hud_schedule() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    state.borrow_mut().character_id = 47;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(super::super::HudBuilt { up: true, fresh: false })
        .add_message::<NetEvent>()
        .add_message::<NetCommand>();
    plugin(&mut app);
    app.world_mut()
        .write_message(NetEvent(Event::CharacterSelected(Box::new(fixture::header()))));
    app.world_mut()
        .write_message(NetEvent(Event::Crime(Receipt::JuryPointChanged { jury_point: 23 })));
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(observed[1].event=='PLAYER_JURY_POINT' and observed[1].jury==23);X2Trial:RequestJuryWaitingNumber()")
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
        [aa_net::Command::RequestJuryWaitingNumber { character_id: 47 }]
    ));
    assert_eq!(state.borrow().crime.received.jury_point, Some(23));
    Ok(())
}
#[test]
#[ignore = "loads original character panel and physically hovers its jury icon"]
fn original_character_labels_hover_and_received_waiting_tooltip() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime(1920., 1080.)?;
    rt.lua
        .load("assert(string.find(CRIME_FIXTURE:GetText(),'60'));assert(JURY_FIXTURE:GetText()=='30')")
        .exec()?;
    let mut mouse = fixture::hover(&rt)?;
    assert!(matches!(
        flush(&mut state.borrow_mut()).as_slice(),
        [aa_net::Command::RequestJuryWaitingNumber { character_id: 47 }]
    ));
    dispatch(Some(&rt), &state, &Event::Crime(Receipt::JuryWaitingNumber { waiting_number: 17 }));
    rt.lua
        .load("assert(X2Player:GetJuryPoint()==31 and tooltip.window:IsVisible())")
        .exec()?;
    let expected: String = rt.lua.load("return locale.trial.wait_jury_tip(17)").eval()?;
    let tip = aa_ui::lua::value_id(&rt.lua, &rt.lua.load("return tooltip.window").eval()?).unwrap();
    assert_eq!(
        aa_ui::lua::ui(&rt.lua).tree.widget(tip).unwrap().tip.as_ref().unwrap().lines[0]
            .left
            .text,
        expected
    );
    aa_ui::input::update(&rt.lua, &mut mouse, Some((0., 0.)), false, false, false, 0.);
    rt.lua.load("assert(not tooltip.window:IsVisible())").exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
