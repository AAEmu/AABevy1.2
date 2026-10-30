//! Native cooldown selection: global first, every descriptor tag in SQL order, then raw skill ID.
//! Global timing still uses the existing successful-cast/DB observation. Ordinary action slots
//! additionally retain their previous own-cast estimate; the constant-button hook uses
//! source-backed manager entries and global timing, including received stop durations.
use std::time::Duration;
use web_time::Instant;

use aa_net::state::Skills;
use bevy::prelude::DetectChangesMut;
use mlua::{Lua, MultiValue, Table};

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

pub(super) fn native_at(info: &SkillInfo, skills: &Skills, id: u32, now: Instant) -> (u32, u32) {
    skills
        .native_cooldown_at(id, &info.tags, now)
        .map_or((0, 0), |(left, total)| (ms(left), ms(total)))
}

fn combined_at(game: &GameData, skills: &Skills, id: u32, now: Instant) -> Option<(u32, u32)> {
    let info = game.skills.get(&id)?;
    let mut best = gcd_at(game, skills, id, now);
    replace(&mut best, native_at(info, skills, id, now));
    Some(best)
}

/// The global cooldown (left, total) a skill waits for: the last successful cast's `gcd_ms` counted from its start, or the
/// one a plot step's `GlobalCooldown` effect started, whichever lasts longer; none for the skills that ignore it.
pub(super) fn gcd_at(game: &GameData, skills: &Skills, id: u32, now: Instant) -> (u32, u32) {
    if game.skills.get(&id).is_none_or(|info| info.ignore_gcd) {
        return (0, 0);
    }
    let mut best = (0, 0);
    if let Some((at, last)) = skills.last_cast.zip(game.skills.get(&skills.last_cast_skill)) {
        let total = last.gcd_ms;
        let left = Duration::from_millis(u64::from(total)).saturating_sub(now.saturating_duration_since(at));
        best = (ms(left), total);
    }
    if let Some((left, total)) = skills.plot_gcd_at(now) {
        replace(&mut best, (ms(left), ms(total)));
    }
    best
}

/// The driven slave's global cooldown (left, total) at `now`: the slave's last accepted cast
/// (`SCSkillStarted`, aa-net's `mount_last_cast`), using that skill's `gcd_ms`.
pub(super) fn mount_gcd_at(
    game: &GameData,
    skills: &Skills,
    caster: u32,
    id: u32,
    now: Instant,
) -> (u32, u32) {
    if game.skills.get(&id).is_none_or(|info| info.ignore_gcd) {
        return (0, 0);
    }
    let Some((at, last)) = skills.mount_last_cast(caster) else {
        return (0, 0);
    };
    let Some(last) = game.skills.get(&last) else {
        return (0, 0);
    };
    let total = last.gcd_ms;
    let left =
        Duration::from_millis(u64::from(total)).saturating_sub(now.saturating_duration_since(at));
    (ms(left), total)
}

/// The driven slave's slots: the same own-cast estimate and global cooldown, read from the
/// slave's receipts (the server sends no `SCCooldowns` for a slave caster, so aa-net stamps
/// `SCSkillStarted`/`SCSkillFired` whose caster is the mount).
pub(super) fn mount_display_at(
    game: &GameData,
    skills: &Skills,
    caster: u32,
    id: u32,
    now: Instant,
) -> Option<(u32, u32)> {
    let info = game.skills.get(&id)?;
    let mut best = mount_gcd_at(game, skills, caster, id, now);
    if let Some((left, total)) = skills.mount_cooldown_at(caster, id, info.cooldown_ms, now) {
        replace(&mut best, (ms(left), ms(total)));
    }
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
    caption_option(lua);
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
            Ok(MultiValue::from_iter([
                crate::lua_int(i64::from(remaining)),
                crate::lua_int(i64::from(total)),
            ]))
        })?,
    )
}

fn caption_option(lua: &Lua) {
    let value = lua
        .load("return X2Option:GetOptionItemValue(OPTION_ITEM_SLOT_COOLDOWN_VISIBLE)")
        .eval::<Option<f32>>();
    if let Ok(Some(value)) = value {
        aa_ui::lua::slot::set_cooldown_numbers(lua, value != 0.0);
    }
}

/// Read the actual account option after original input handlers and before slot drawing.
pub(super) fn sync_caption_option(rt: Option<bevy::prelude::NonSend<aa_ui::UiRuntime>>) {
    if let Some(rt) = rt {
        caption_option(&rt.lua);
    }
}

/// Bind actual descriptor routing before network PreUpdate; zero primary tags are meaningful.
/// The network resource preserves this static catalog across character and connection resets.
pub(super) fn bind_catalog(
    mut bound: bevy::prelude::Local<bool>,
    bridge: bevy::prelude::NonSend<super::api::HudBridge>,
    skills: Option<bevy::prelude::ResMut<Skills>>,
) {
    if *bound {
        return;
    }
    let data = bridge.0.borrow();
    let Some(game) = data.game.get() else { return };
    let Some(mut skills) = skills else { return };
    skills.set_cooldown_catalog(game.skills.iter().map(|(&id, info)| (id, info.tag)).collect());
    skills.set_plot_gcd_catalog(game.plot_gcd.clone());
    *bound = true;
}

#[allow(dead_code)]
pub(super) mod fixture;
#[cfg(test)]
mod tests;

/// Queued network receipts are folded in PreUpdate. Sample the process frame clock once,
/// before any UI input/tick or HUD snapshot reads the native manager's stored rows.
pub(super) fn advance_frame(
    time: bevy::prelude::Res<bevy::prelude::Time<bevy::time::Real>>,
    skills: Option<bevy::prelude::ResMut<Skills>>,
) {
    let Some(mut skills) = skills else { return };
    let now = time.elapsed().as_millis() as u32;
    if skills.bypass_change_detection().advance_native_frame(now) {
        skills.set_changed();
    }
    // Clock-only changes must survive lifecycle resets too, but are not a new server receipt.
}
