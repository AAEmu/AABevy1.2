//! Native cooldown selection: global first, every descriptor tag in SQL order, then raw skill ID.
//! Global timing still uses the existing successful-cast/DB observation. Ordinary action slots
//! additionally retain their previous own-cast estimate; the constant-button hook uses received
//! entries and global timing only, without inventing a received own cooldown.
use std::time::{Duration, Instant};

use aa_net::state::Skills;
use mlua::{Lua, MultiValue, Table, Value};

use super::api::Shared;
use super::data::{GameData, SkillInfo};

fn ms(value: Duration) -> u32 {
    value.as_millis().min(u128::from(u32::MAX)) as u32
}

fn replace(best: &mut (u32, u32), candidate: (u32, u32)) {
    if candidate.0 > best.0 {
        *best = candidate;
    }
}

pub(super) fn received_at(info: &SkillInfo, skills: &Skills, id: u32, now: Instant) -> (u32, u32) {
    let mut best = (0, 0);
    for entry in info
        .tags
        .iter()
        .flat_map(|tag| skills.received_tag_cooldowns.iter().filter(move |entry| entry.cooldown.id == *tag))
        .chain(skills.received_skill_cooldowns.iter().filter(|entry| entry.cooldown.id == id))
    {
        if let Some(left) = entry.left_at(now) {
            replace(&mut best, (ms(left), entry.cooldown.total_ms));
        }
    }
    best
}

fn combined_at(game: &GameData, skills: &Skills, id: u32, now: Instant) -> Option<(u32, u32)> {
    let info = game.skills.get(&id)?;
    let mut best = (0, 0);
    if !info.ignore_gcd {
        if let Some((at, last)) = skills.last_cast.zip(game.skills.get(&skills.last_cast_skill)) {
            let total = last.gcd_ms;
            let left = Duration::from_millis(u64::from(total)).saturating_sub(now.saturating_duration_since(at));
            best = (ms(left), total);
        }
    }
    replace(&mut best, received_at(info, skills, id, now));
    Some(best)
}

/// Ordinary slots retain the existing own-cast estimate separately from received manager entries.
pub(super) fn display_at(game: &GameData, skills: &Skills, id: u32, now: Instant) -> Option<(u32, u32)> {
    let mut best = combined_at(game, skills, id, now)?;
    let info = game.skills.get(&id)?;
    replace(&mut best, observed_at(info, skills, id, now));
    Some(best)
}

/// Existing cast-start/DB estimate only; never reintroduce the single tag as received membership.
pub(super) fn observed_at(info: &SkillInfo, skills: &Skills, id: u32, now: Instant) -> (u32, u32) {
    if let Some(&used) = skills.last_used.get(&id) {
        let reset = info.tag != 0 && skills.reset_tags.iter().any(|&(tag, at)| tag == info.tag && at >= used);
        if !reset {
            let left = Duration::from_millis(u64::from(info.cooldown_ms)).saturating_sub(now.saturating_duration_since(used));
            return (ms(left), info.cooldown_ms);
        }
    }
    (0, 0)
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let aaui: Table = lua.globals().get("AAUI")?;
    let state = bridge.clone();
    aaui.set(
        "SkillCooldown",
        lua.create_function(move |_, id: u32| {
            let data = state.borrow();
            let Some(game) = data.game.get().filter(|game| game.skills.contains_key(&id)) else {
                return Ok(MultiValue::new());
            };
            let (remaining, total) = if data.cooldowns.obj_id == 0 {
                (0, 0)
            } else {
                combined_at(game, &data.cooldowns, id, Instant::now()).unwrap_or_default()
            };
            Ok(MultiValue::from_iter([Value::Integer(i64::from(remaining)), Value::Integer(i64::from(total))]))
        })?,
    )
}

#[allow(dead_code)]
pub(super) mod fixture;
#[cfg(test)]
mod tests;
