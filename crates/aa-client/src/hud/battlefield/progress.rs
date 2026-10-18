//! Original received battlefield getter fields; clock-dependent tables require an actual clock.
use super::*;
use aa_net::battlefield::BattlefieldPhase;

pub(super) fn corps_name(data: &HudData, corps: u8) -> String {
    if corps == 2 {
        return "ANY_CORPS".into();
    }
    (|| {
        let game = data.game.get()?;
        let (_, rule) = game.battlefield.rule(data.battlefield.received.joined_ruleset_id?)?;
        game.factions.get(rule.factions.get(usize::from(corps))?).cloned()
    })()
    .unwrap_or_default()
}

fn victory(lua: &Lua, data: &HudData) -> mlua::Result<Option<Table>> {
    let received = &data.battlefield.received;
    let Some(joined) = received.joined.filter(|_| received.in_instant_game()) else {
        return Ok(None);
    };
    let out = lua.create_table()?;
    out.set("victoryScore", number(joined.victory_score))?;
    out.set("victoryKillCount", number(i32::from(joined.victory_kill_count)))?;
    out.set("victoryTarget", Value::Number(f64::from(joined.victory_kill_corps_head as f32)))?;
    Ok(Some(out))
}

fn score(lua: &Lua, data: &HudData) -> mlua::Result<Option<Table>> {
    let received = &data.battlefield.received;
    let Some(joined) = received.joined.filter(|_| received.in_instant_game()) else {
        return Ok(None);
    };
    let out = lua.create_table()?;
    out.set("myKillStreakPoint", number(received.my_killstreak_point))?;
    let team = received.result.as_ref().and_then(|result| result.teams.get(usize::from(joined.corps)));
    out.set("myCorpsVictoryState", number(team.map_or(0, |team| i32::from(team.state))))?;
    out.set("myCorpsKill", number(team.map_or(0, |team| i32::from(team.total_kills as u16))))?;
    out.set("myCorpsDeath", number(team.map_or(0, |team| i32::from(team.total_deaths as u16))))?;
    for (key, value) in [
        ("score1", received.scores[0]),
        ("score2", received.scores[1]),
        ("killScore1", received.kill_scores[0]),
        ("killScore2", received.kill_scores[1]),
    ] {
        out.set(key, number(value))?;
    }
    Ok(Some(out))
}

pub(super) fn time_at(lua: &Lua, data: &HudData, now: u32) -> mlua::Result<Option<Table>> {
    let received = &data.battlefield.received;
    let Some((phase, joined)) = received.phase.zip(received.joined) else {
        return Ok(None);
    };
    let state = match phase {
        BattlefieldPhase::Joined => "JOINED",
        BattlefieldPhase::Opening => "COUNTDOWN",
        BattlefieldPhase::Playing => "STARTED",
        BattlefieldPhase::Ending => "ENDED",
    };
    let bounds = match phase {
        BattlefieldPhase::Joined => None,
        BattlefieldPhase::Opening => match received.opening_at_ms.zip(received.start) {
            Some((start, end)) => Some((start, end.start_ms)),
            None => return Ok(None),
        },
        BattlefieldPhase::Playing => match received.start {
            Some(start) => Some((start.start_ms, start.start_ms.wrapping_add(joined.phase_times_ms[1] as u32))),
            None => return Ok(None),
        },
        BattlefieldPhase::Ending => match received.ending_at_ms {
            Some(start) => Some((start, start.wrapping_add(joined.phase_times_ms[2] as u32))),
            None => return Ok(None),
        },
    };
    let out = lua.create_table()?;
    out.set("state", state)?;
    let now = if phase == BattlefieldPhase::Opening { now.wrapping_add(6000) } else { now };
    out.set("nowtime", Value::Number(f64::from(now as f32)))?;
    if let Some((start, end)) = bounds {
        out.set("startTime", Value::Number(f64::from(start as f32)))?;
        out.set("endTime", Value::Number(f64::from(end as f32)))?;
    }
    Ok(Some(out))
}

fn time(lua: &Lua, data: &HudData) -> mlua::Result<Option<Table>> {
    let Some(now) = data.battlefield.clock.mapped_now() else { return Ok(None) };
    time_at(lua, data, now)
}

fn entire(lua: &Lua, data: &HudData) -> mlua::Result<Option<Table>> {
    let Some(out) = time(lua, data)? else { return Ok(None) };
    let received = &data.battlefield.received;
    let joined = received.joined.unwrap();
    out.set("corpsName", corps_name(data, joined.corps))?;
    for (i, key) in ["corpsName1", "corpsName2"].into_iter().enumerate() {
        out.set(
            key,
            data.game.get().and_then(|g| g.factions.get(&joined.factions[i])).map_or("", String::as_str),
        )?;
    }
    out.set("endReason", number(received.result.as_ref().map_or(0, |result| i32::from(result.reason))))?;
    for extra in [score(lua, data)?, victory(lua, data)?].into_iter().flatten() {
        for pair in extra.pairs::<Value, Value>() {
            let (key, value) = pair?;
            out.raw_set(key, value)?;
        }
    }
    Ok(Some(out))
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2BattleField")?;
    let state = bridge.clone();
    api.set(
        "GetMyKillstreakSkillsInfos",
        lua.create_function(move |lua, _: Value| {
            let data = state.borrow();
            let received = &data.battlefield.received;
            let ids = received.joined.map_or([0; 24], |joined| joined.killstreak_skills);
            let out = lua.create_table()?;
            out.set("skill_count", number(ids.iter().filter(|id| **id != 0).count() as i32))?;
            for (index, id) in ids.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.set("skill_type", Value::Number(f64::from(id as f32)))?;
                row.set("enable", received.skills_enabled[index])?;
                out.raw_set(index + 1, row)?;
            }
            Ok(out)
        })?,
    )?;
    for (name, get) in [
        ("GetProgressVictoryConditionInfo", victory as fn(&Lua, &HudData) -> mlua::Result<Option<Table>>),
        ("GetProgressScoreInfo", score),
        ("GetProgressTimeInfo", time),
        ("GetProgressEntireInfo", entire),
    ] {
        let state = bridge.clone();
        api.set(
            name,
            lua.create_function(move |lua, _: Value| {
                Ok(get(lua, &state.borrow())?.map_or_else(MultiValue::new, |value| MultiValue::from_iter([Value::Table(value)])))
            })?,
        )?;
    }
    let state = bridge.clone();
    api.set(
        "GetMembersDetailInfo",
        lua.create_function(move |lua, _: Value| {
            let data = state.borrow();
            let received = &data.battlefield.received;
            if !received.in_instant_game() {
                return Ok(MultiValue::new());
            }
            let out = lua.create_table()?;
            let mut own_index = 0;
            let factor = if received.my_corps_kill_death_ratio > 0 {
                (received.my_corps_kill_death_ratio / 100) as f32
            } else {
                1f32
            };
            for team in 0..2 {
                let members = lua.create_table()?;
                if let Some(result) = &received.result {
                    for (index, member) in result.teams[team].members.iter().take(50).enumerate() {
                        let row = lua.create_table()?;
                        row.set("name", member.name.as_str())?;
                        row.set("memberScores", f64::from(member.score as f32 * factor))?;
                        row.set("memberBonus", f64::from(member.bonus as f32 * factor))?;
                        for (key, value) in [
                            ("memberKillstreaks", member.killstreak),
                            ("memberNPcKills", member.kills),
                            ("memberNPcKillAssists", member.assists),
                            ("memberNDeaths", member.deaths),
                        ] {
                            row.set(key, number(i32::from(value)))?;
                        }
                        for (bit, key) in ["isWinner", "noDeath", "topPcKill", "topNonPcKill", "isLoser"].into_iter().enumerate() {
                            row.set(key, member.bonus_set & (1 << bit) != 0)?;
                        }
                        if received.local_character_id == Some(member.character_id) {
                            own_index = index;
                        }
                        members.raw_set(index + 1, row)?;
                    }
                }
                out.raw_set(team + 1, members)?;
            }
            Ok(MultiValue::from_iter([Value::Table(out), number(own_index as i32)]))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "AskLeaveInstantGame",
        lua.create_function(move |_, _: Value| {
            let mut data = state.borrow_mut();
            let owner = data.character_id;
            if owner != 0 && data.battlefield.owner == owner && data.battlefield.received.validate_command(BattlefieldCommand::Leave).is_ok() {
                data.battlefield.requests.push((owner, 0, BattlefieldCommand::Leave));
            }
            Ok(())
        })?,
    )?;
    Ok(())
}
