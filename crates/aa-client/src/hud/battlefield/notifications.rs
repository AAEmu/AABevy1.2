//! Native receive ordering: local killstreak notification precedes the guarded match update.
use super::*;
use aa_net::battlefield::BattlefieldPhase;

pub(super) struct Before {
    pub phase: Option<BattlefieldPhase>,
    point: bool,
    kill: bool,
    skill: bool,
}

pub(super) fn before_receive(rt: &UiRuntime, bridge: &Shared, event: &Event) -> Before {
    if let Event::Battlefield(BattlefieldEvent::Kill(kill)) = event {
        let notify = {
            let mut data = bridge.borrow_mut();
            let received = &mut data.battlefield.received;
            let notify = received.local_killstreak_notification(kill);
            if notify.is_some() {
                received.my_killstreak_point = i32::from(kill.killer_killstreak);
            }
            notify
        };
        if let Some(value) = notify {
            rt.fire_event("UPDATE_INSTANT_GAME_KILLSTREAK_COUNT", (f64::from(value as f32),));
        }
    }
    let data = bridge.borrow();
    let received = &data.battlefield.received;
    Before {
        phase: received.phase,
        point: matches!(event, Event::Battlefield(BattlefieldEvent::Point(point)) if received.accepts_point(point)),
        kill: matches!(event, Event::Battlefield(BattlefieldEvent::Kill(kill)) if received.accepts_kill(kill)),
        skill: matches!(event, Event::Battlefield(BattlefieldEvent::SkillEnable(skill)) if received.accepts_skill_change(skill)),
    }
}

pub(super) fn after_receive(rt: &UiRuntime, bridge: &Shared, event: &Event, before: Before) {
    let phase = bridge.borrow().battlefield.received.phase;
    if before.phase == Some(BattlefieldPhase::Joined) && phase == Some(BattlefieldPhase::Opening) {
        rt.fire_event("INSTANT_GAME_WAIT", ());
        let started = {
            let mut data = bridge.borrow_mut();
            let view = &mut data.battlefield;
            if view
                .clock
                .mapped_now()
                .is_some_and(|now| view.received.start.is_some_and(|start| start.start_ms <= now))
            {
                view.received.phase = Some(BattlefieldPhase::Playing);
                true
            } else {
                false
            }
        };
        if started {
            rt.fire_event("INSTANT_GAME_START", ());
        }
    }
    if before.phase == Some(BattlefieldPhase::Playing) && phase == Some(BattlefieldPhase::Ending) {
        rt.fire_event("INSTANT_GAME_END", ());
    }
    match event {
        Event::Battlefield(BattlefieldEvent::UnearnedWinRemaining { milliseconds }) => {
            let seconds = aa_net::battlefield::unearned_win_remaining_seconds(*milliseconds);
            rt.fire_event("INSTANT_GAME_UNEARNED_WIN_REMAIN_TIME", (f64::from(seconds as f32),));
        }
        Event::Battlefield(BattlefieldEvent::Point(point)) if before.point => {
            let corps = super::progress::corps_name(&bridge.borrow(), point.corps);
            rt.fire_event("INSTANT_GAME_ADD_POINT", (corps, point.character_name.clone(), f64::from(point.points as f32)));
        }
        Event::Battlefield(BattlefieldEvent::SkillEnable(_)) if before.skill => {
            rt.fire_event("UPDATE_INSTANT_GAME_KILLSTREAK", ());
        }
        Event::Battlefield(BattlefieldEvent::Kill(kill)) if before.kill => {
            let result = (|| -> mlua::Result<Table> {
                let data = bridge.borrow();
                let table = rt.lua.create_table()?;
                table.set("killerCorps", super::progress::corps_name(&data, kill.killer_corps))?;
                table.set("killer", kill.killer_name.as_str())?;
                table.set("victimCorps", super::progress::corps_name(&data, kill.victim_corps))?;
                table.set("victim", kill.victim_name.as_str())?;
                table.set("killerKillstreak", number(i32::from(kill.killer_killstreak)))?;
                table.set("killerCorpsKill", number(kill.killer_corps_kills))?;
                table.set("victimCorpsDeath", number(kill.victim_corps_deaths))?;
                Ok(table)
            })();
            match result {
                Ok(table) => rt.fire_event("INSTANT_GAME_KILL", (table,)),
                Err(error) => warn!("hud: battlefield kill notification: {error}"),
            }
        }
        _ => {}
    }
}

pub(super) fn advance_at(rt: &UiRuntime, bridge: &Shared, now: u32) {
    let started = bridge.borrow_mut().battlefield.received.advance_server_time(now);
    if started {
        rt.fire_event("INSTANT_GAME_START", ());
    }
}

pub(super) fn advance(rt: &UiRuntime, bridge: &Shared) {
    let now = bridge.borrow().battlefield.clock.mapped_now();
    if let Some(now) = now {
        advance_at(rt, bridge, now);
    }
}
