use super::*;
use aa_net::DuelEvent;

fn setup() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let (rt, state, _) = super::super::breath::fixture::runtime(1280., 720.)?;
    rt.lua
        .load(
            r#"
        observed = {}
        local listener = UIParent:CreateWidget('emptywidget', 'duelListener', 'UIParent')
        listener:SetHandler('OnEvent', function(self, event, ...) table.insert(observed, event) end)
        listener:RegisterEvent('STARTED_DUEL'); listener:RegisterEvent('ENDED_DUEL')
        local request = X2DialogManager.RequestDefaultDialog
        X2DialogManager.RequestDefaultDialog = function(self, fn, owner)
          local wnd = request(self, fn, owner)
          if owner == 'duelChallenge' then CHALLENGE = wnd end
          return wnd
        end
        local notice = X2DialogManager.RequestNoticeDialog
        X2DialogManager.RequestNoticeDialog = function(self, fn, owner)
          local wnd = notice(self, function(w)
            local set = w.SetContent
            w.SetContent = function(s, text) if owner == 'duelResult' then CONTENT = text end return set(s, text) end
            fn(w)
          end, owner)
          if owner == 'duelResult' then RESULT = wnd end
          return wnd
        end
    "#,
        )
        .exec()?;
    state.borrow_mut().character_id = 10;
    Ok((rt, state))
}

fn send(rt: &UiRuntime, state: &Shared, event: DuelEvent) {
    dispatch(Some(rt), state, &Event::Duel(event), &Names::default());
}

fn requests(state: &Shared) -> Vec<aa_net::Command> {
    std::mem::take(&mut state.borrow_mut().duel.requests)
}

#[test]
fn a_challenge_prompts_and_the_answer_goes_to_the_server() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    send(&rt, &state, DuelEvent::Challenged { challenger_id: 20 });
    rt.lua.load("assert(CHALLENGE and CHALLENGE:IsVisible())").exec()?;
    crate::hud::expansion::fixture::click(&rt, "CHALLENGE.btnOk")?;
    assert!(matches!(requests(&state)[..], [aa_net::Command::AnswerDuel { challenger_id: 20, accept: true }]));
    // answered once: a second click cannot answer again
    rt.lua.load("CHALLENGE.OkProc()").exec()?;
    assert!(requests(&state).is_empty());

    send(&rt, &state, DuelEvent::Challenged { challenger_id: 21 });
    crate::hud::expansion::fixture::click(&rt, "CHALLENGE.btnCancel")?;
    assert!(matches!(requests(&state)[..], [aa_net::Command::AnswerDuel { challenger_id: 21, accept: false }]));
    Ok(())
}

#[test]
fn the_start_and_the_end_reach_the_scripts_and_the_result_is_shown() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    state.borrow_mut().duel.received.character_id = Some(10);
    send(&rt, &state, DuelEvent::Challenged { challenger_id: 20 });
    send(&rt, &state, DuelEvent::Started { challenger_obj_id: 5, challenged_obj_id: 6 });
    send(&rt, &state, DuelEvent::StartCountdown);
    assert!(state.borrow().duel.received.active.is_some());
    // the started duel withdrew the open prompt
    rt.lua.load("assert(not CHALLENGE:IsVisible())").exec()?;
    send(&rt, &state, DuelEvent::Ended { winner_id: 10, loser_id: 20, winner_obj_id: 6, loser_obj_id: 5, det: 1 });
    rt.lua
        .load("assert(observed[1]=='STARTED_DUEL' and observed[2]=='ENDED_DUEL' and #observed==2); assert(RESULT and RESULT:IsVisible())")
        .exec()?;
    assert!(state.borrow().duel.received.active.is_none());
    Ok(())
}

#[test]
fn the_popup_action_challenges_a_character_target_only() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua.load("assert(not X2Unit:CanChallengeTargetToDuel()); X2Unit:ChallengeTargetToDuel()").exec()?;
    assert!(requests(&state).is_empty());
    let target = |unit_type: &'static str, character_id: u32, hp: i32| super::super::panels::TargetSnap {
        unit_type,
        character_id,
        hp,
        ..Default::default()
    };
    for (snap, can) in [
        (target("character", 30, 100), true),
        (target("npc", 0, 100), false),
        (target("character", 10, 100), false),
        (target("character", 30, 0), false),
    ] {
        state.borrow_mut().panels.target = Some(snap);
        rt.lua.load(format!("assert(X2Unit:CanChallengeTargetToDuel() == {can})")).exec()?;
    }
    state.borrow_mut().panels.target = Some(target("character", 30, 100));
    rt.lua.load("X2Unit:ChallengeTargetToDuel()").exec()?;
    assert!(matches!(requests(&state)[..], [aa_net::Command::ChallengeDuel { challenged_id: 30 }]));
    // no second challenge while one waits for its answer
    send(&rt, &state, DuelEvent::Challenged { challenger_id: 20 });
    rt.lua.load("assert(not X2Unit:CanChallengeTargetToDuel())").exec()?;
    Ok(())
}

#[test]
fn ignored_invitations_are_refused_without_a_prompt() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua.load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_DUEL_INVITATION,1)").exec()?;
    send(&rt, &state, DuelEvent::Challenged { challenger_id: 20 });
    assert!(matches!(requests(&state)[..], [aa_net::Command::AnswerDuel { challenger_id: 20, accept: false }]));
    rt.lua.load("assert(CHALLENGE == nil)").exec()?;
    assert!(state.borrow().duel.received.challenge.is_none());
    // switched off again: the next challenge is prompted
    rt.lua.load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_DUEL_INVITATION,0)").exec()?;
    send(&rt, &state, DuelEvent::Challenged { challenger_id: 21 });
    rt.lua.load("assert(CHALLENGE and CHALLENGE:IsVisible())").exec()?;
    assert!(requests(&state).is_empty());
    Ok(())
}

#[test]
fn the_results_are_kept_for_the_script_runner() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    state.borrow_mut().duel.received.character_id = Some(10);
    let bridge = super::super::api::HudBridge(state.clone());
    send(&rt, &state, DuelEvent::Started { challenger_obj_id: 5, challenged_obj_id: 6 });
    assert!(super::super::script_api::facts(&bridge).duel_active);
    send(&rt, &state, DuelEvent::Ended { winner_id: 20, loser_id: 10, winner_obj_id: 6, loser_obj_id: 5, det: 1 });
    send(&rt, &state, DuelEvent::Started { challenger_obj_id: 5, challenged_obj_id: 6 });
    send(&rt, &state, DuelEvent::Ended { winner_id: 20, loser_id: 10, winner_obj_id: 6, loser_obj_id: 5, det: 2 });
    let facts = super::super::script_api::facts(&bridge);
    assert!(!facts.duel_active);
    assert_eq!(facts.duel_results, ["result_loser", "result_runaway"]);
    Ok(())
}

#[test]
fn the_press_helper_answers_the_open_prompt() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    send(&rt, &state, DuelEvent::Challenged { challenger_id: 20 });
    rt.lua.load("HUD.duel_press(true)").exec()?;
    assert!(matches!(requests(&state)[..], [aa_net::Command::AnswerDuel { challenger_id: 20, accept: true }]));
    rt.lua.load("assert(not CHALLENGE:IsVisible())").exec()?;
    Ok(())
}

#[test]
fn the_countdown_announces_three_two_one_and_ends_with_the_start() -> Result<(), Box<dyn std::error::Error>> {
    let mut countdown = Countdown::default();
    let mut said = Vec::new();
    let mut ended = false;
    for _ in 0..40 {
        let (now, over) = countdown.advance(0.1);
        said.extend(now);
        ended |= over;
    }
    assert_eq!(said, [3, 2, 1]);
    assert!(ended);

    let (rt, state) = setup()?;
    send(&rt, &state, DuelEvent::StartCountdown);
    assert!(state.borrow().duel.countdown.is_some());
    rt.lua
        .load(r#"assert(HUD.duel_countdown_text(3) == "Duel will begin in 3 sec.", HUD.duel_countdown_text(3))"#)
        .exec()?;
    // the start (and the end) cuts a running countdown short
    send(&rt, &state, DuelEvent::Started { challenger_obj_id: 5, challenged_obj_id: 6 });
    assert!(state.borrow().duel.countdown.is_none());
    // a countdown that arrives after the start is not run
    send(&rt, &state, DuelEvent::StartCountdown);
    assert!(state.borrow().duel.countdown.is_none());
    Ok(())
}

#[test]
fn a_surrender_shows_the_texts_of_both_sides() -> Result<(), Box<dyn std::error::Error>> {
    for (own, text) in [(20, "Surrendered."), (10, "Opponent surrendered.")] {
        let (rt, state) = setup()?;
        state.borrow_mut().duel.received.character_id = Some(own);
        send(&rt, &state, DuelEvent::Started { challenger_obj_id: 5, challenged_obj_id: 6 });
        send(&rt, &state, DuelEvent::Ended { winner_id: 10, loser_id: 20, winner_obj_id: 5, loser_obj_id: 6, det: 2 });
        rt.lua.load(format!("assert(RESULT and RESULT:IsVisible()); assert(CONTENT == {text:?}, CONTENT)")).exec()?;
    }
    Ok(())
}
