//! The HUD's rows of the skill trace (`AA_SKILL_TRACE`, `crate::skilltrace`): the cast bars and the cooldowns, read from the
//! state the Lua UI reads, and written when they change.
//!
//! * `castbar`: `X2Unit:UnitCastingInfo("player" / "target")` is `HUD.casting_info` over [`super::cast::Casts`]; a row when a
//!   bar appears (`start`), disappears (`stop`) or turns into another (a cast that becomes a channel: `stop`, then `start`).
//! * `cooldown`: the cooldown an action bar skill slot shows is [`super::panels::action_skill_cooldown`] (the number the HUD
//!   quantises to a quarter second for the slot; here as it is), and the global cooldown pair is [`super::cooldown::global_at`].
//!   A row when one starts or is restarted, and one with `remain_ms` 0 when it ends. Display slots 1..=12 of the current page.

use aa_net::items::ActionKind;
use aa_net::state::{ActionBar, Inventory, Skills};
use bevy::prelude::*;
use web_time::Instant;

use super::api::HudBridge;
use super::cast::CastView;
use crate::skilltrace::{self, rows};

/// The cast bars of the player and the target, as `X2Unit:UnitCastingInfo` answers.
pub(crate) fn castbar(bridge: Option<NonSend<HudBridge>>, mut last: Local<[Option<CastView>; 2]>) {
    let Some(bridge) = bridge else { return };
    let data = bridge.0.borrow();
    for (i, unit) in ["player", "target"].into_iter().enumerate() {
        let now = data.casts.trace_view(unit);
        let prev = last[i].take();
        let changed = match (&prev, &now) {
            (None, None) => false,
            (Some(a), Some(b)) => (&a.name, a.total_ms, a.channel) != (&b.name, b.total_ms, b.channel),
            _ => true,
        };
        if changed {
            if let Some(p) = &prev {
                emit(unit, "stop", p);
            }
            if let Some(n) = &now {
                emit(unit, "start", n);
            }
        }
        last[i] = now;
    }
}

fn emit(unit: &str, state: &str, bar: &CastView) {
    skilltrace::emit("castbar", |r| {
        r.str("unit", unit)
            .str("state", state)
            .str("name", &rows::name(&bar.name))
            .int("total_ms", i64::from(bar.total_ms))
            .int("elapsed_ms", i64::from(bar.elapsed_ms))
            .flag("channel", bar.channel)
    });
}

/// A cooldown that is running: when it ends (wall clock, ms) and how long it lasts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Running {
    end_ms: i64,
    total_ms: u32,
}

/// How far the end of a cooldown may move between frames before it counts as restarted (a frame, with room for the timing noise).
const RESTART_MS: i64 = 100;

/// The row a pair of samples of one cooldown calls for: the cooldown it is about and whether it is running (a start or a
/// restart) or over (the end, written with the length it had).
fn change(prev: Option<Running>, now: Option<Running>) -> Option<(Running, bool)> {
    match (prev, now) {
        (None, None) => None,
        (None, Some(n)) => Some((n, true)),
        (Some(p), None) => Some((p, false)),
        (Some(p), Some(n)) => (p.total_ms != n.total_ms || (p.end_ms - n.end_ms).abs() > RESTART_MS).then_some((n, true)),
    }
}

/// The global cooldown and the cooldowns of the twelve action bar slots shown.
pub(crate) fn cooldowns(
    bridge: Option<NonSend<HudBridge>>,
    bar: Option<Res<ActionBar>>,
    skills: Option<Res<Skills>>,
    inventory: Option<Res<Inventory>>,
    mut last: Local<[Option<Running>; 13]>,
) {
    let (Some(bridge), Some(skills)) = (bridge, skills) else { return };
    let data = bridge.0.borrow();
    let shared = data.game.clone();
    let game = shared.get();
    let (now, wall_ms) = (Instant::now(), (skilltrace::wall_us() / 1000) as i64);
    let running = |cd: Option<(u32, u32)>| cd.filter(|(left, _)| *left > 0).map(|(left, total)| Running { end_ms: wall_ms + i64::from(left), total_ms: total });
    // [0] the global cooldown, [1..=12] the display slots
    let mut sample: [Option<Running>; 13] = [None; 13];
    sample[0] = running(game.map(|g| super::cooldown::global_at(g, &skills, now)));
    if let Some(bar) = bar {
        for (display, cell) in sample.iter_mut().enumerate().skip(1) {
            let slot = ActionBar::display_slot_on(data.action_page, display - 1).and_then(|s| bar.slots.get(s));
            if let Some(slot) = slot.filter(|s| !s.is_empty() && matches!(s.kind, ActionKind::Spell | ActionKind::RidePetSpell)) {
                *cell = running(super::panels::action_skill_cooldown(game, Some(&skills), inventory.as_deref(), slot.id as u32, now));
            }
        }
    }
    for (i, now) in sample.into_iter().enumerate() {
        // an unchanged cooldown keeps its first sample as the reference, so the rows are edges and not a stream
        let Some((cooldown, on)) = change(last[i], now) else { continue };
        let remain = if on { (cooldown.end_ms - wall_ms).max(0) } else { 0 };
        skilltrace::emit("cooldown", |r| {
            r.opt_int("slot", (i > 0).then_some(i as i64)).int("remain_ms", remain).int("total_ms", i64::from(cooldown.total_ms)).flag("gcd", i == 0)
        });
        last[i] = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(end_ms: i64, total_ms: u32) -> Option<Running> {
        Some(Running { end_ms, total_ms })
    }

    #[test]
    fn a_cooldown_writes_a_row_at_its_start_its_restart_and_its_end() {
        assert_eq!(change(None, None), None);
        assert_eq!(change(None, running(5_000, 1_000)), Some((running(5_000, 1_000).unwrap(), true)), "start");
        // the same cooldown a frame later: the end has not moved
        assert_eq!(change(running(5_000, 1_000), running(5_004, 1_000)), None);
        // pressed again: the end moved on
        assert!(matches!(change(running(5_000, 1_000), running(6_200, 1_000)), Some((_, true))), "restart");
        // another length, same end
        assert!(matches!(change(running(5_000, 1_000), running(5_000, 1_500)), Some((_, true))));
        assert_eq!(change(running(5_000, 1_000), None), Some((running(5_000, 1_000).unwrap(), false)), "the end keeps the length it had");
    }
}
