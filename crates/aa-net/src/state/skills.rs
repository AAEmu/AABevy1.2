//! Skills: what the player knows, what units are casting, and when skills were last used.
//!
//! Cast observations use the game's database lengths. Native `SCCooldowns` can additionally
//! supply explicit skill/tag duration and remaining-time lists; the current lane only declares
//! an empty writer and has no emitter. Received lists append without resetting existing timers.

use std::collections::HashMap;
use std::time::{Duration, Instant};

mod native_timers;
pub use native_timers::{NativeCooldown, NativeCooldownOrigin};

use crate::session::Event;
use crate::skill::{Cooldown, SkillTarget, COOLDOWN_LIMIT};

/// A skill the player has learned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownSkill {
    pub id: u32,
    pub level: u8,
}

/// A cast (or channel) in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Casting {
    pub skill_id: u32,
    /// Timeline id of the cast.
    pub tl: u16,
    pub target: SkillTarget,
    /// Cast time in ms (0 = instant), with haste.
    pub cast_ms: u32,
    pub base_cast_ms: u32,
    pub started: Instant,
    /// When the cast completed (`SkillFired`).
    pub fired: Option<Instant>,
    /// Channeling time in ms after the cast, 0 for plain skills.
    pub channeling_ms: i32,
}

impl Casting {
    /// The cast bar: 0.0 at the start, 1.0 when the cast time has passed (1.0 at once for an
    /// instant skill).
    pub fn progress(&self) -> f32 {
        if self.cast_ms == 0 || self.fired.is_some() {
            return 1.0;
        }
        (self.started.elapsed().as_secs_f32() * 1000.0 / self.cast_ms as f32).min(1.0)
    }
}

/// The last skill the server refused to start.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillFailure {
    pub skill_id: u32,
    /// `skill::result::*`.
    pub result: u8,
    pub value: u32,
    pub at: Instant,
}

/// An original received entry, its local reception time, and any authoritative reset.
/// Values are retained after expiry/reset; this is not an invented cast or learned skill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceivedCooldown {
    pub cooldown: Cooldown,
    pub received_at: Instant,
    /// A later own `SkillCooldownReset` cleared this entry. Original wire values stay intact.
    pub reset: bool,
}

impl ReceivedCooldown {
    pub fn left_at(&self, now: Instant) -> Option<Duration> {
        if self.reset {
            return None;
        }
        Duration::from_millis(u64::from(self.cooldown.remaining_ms))
            .checked_sub(now.saturating_duration_since(self.received_at))
            .filter(|left| !left.is_zero())
    }
}

resource! {
    /// Skills of the local player and the casts in view.
    pub struct Skills {
        /// The player's object id (0 until the character spawned).
        pub obj_id: u32,
        /// Learned skills (the server lists them when the character spawns).
        pub known: Vec<KnownSkill>,
        /// Passive buffs the player learned.
        pub passive_buffs: Vec<u32>,
        /// Casts in progress by caster object id (the player included).
        pub casting: HashMap<u32, Casting>,
        /// When the player last started each skill.
        pub last_used: HashMap<u32, Instant>,
        /// When the player last started any skill (for the global cooldown) and which skill it was
        /// (the length of the global cooldown is that skill's `custom_gcd`, or 1 s with `default_gcd`).
        pub last_cast: Option<Instant>,
        pub last_cast_skill: u32,
        /// Cooldown tags reset by the server (`SkillCooldownReset::tag_id`) and when: a skill with
        /// that `cooldown_tag_id` used before that moment is ready again.
        pub reset_tags: Vec<(u32, Instant)>,
        /// Received native lists in arrival order, including duplicates and expired/reset entries.
        /// Each list stops accepting entries at the native capacity of 150.
        pub received_skill_cooldowns: Vec<ReceivedCooldown>,
        pub received_tag_cooldowns: Vec<ReceivedCooldown>,
        /// Ordered mutable native manager rows seeded/extended only by received reports.
        pub native_skill_cooldowns: Vec<NativeCooldown>,
        pub native_tag_cooldowns: Vec<NativeCooldown>,
        /// Local GM mode byte1 established by matching SC203 only. The spawn's
        /// legacy PISH projection is not proof of the effective native actor byte.
        pub native_gm_mode1: Option<u8>,
        /// Native process frame timer; survives actor and connection resets.
        pub native_last_frame_ms: u32,
        /// Static skill?primary cooldown tag routing; prefer set_cooldown_catalog.
        pub cooldown_catalog: std::sync::Arc<HashMap<u32,u32>>,
        /// The last refused skill (not enough mana, too far ...).
        pub last_failure: Option<SkillFailure>,
        /// Static plot step -> global cooldown ms: the plot steps with a `GlobalCooldown` special effect (type 41). The
        /// server runs it in the middle of a plot (Triple Slash: 2 s at its finishing step) and sends nothing but the
        /// step's `PlotEvent`, so the client applies it itself. Set with `set_plot_gcd_catalog`.
        pub plot_gcd_catalog: std::sync::Arc<HashMap<u32, u32>>,
        /// The global cooldown an own plot step started: (when, ms).
        pub plot_gcd: Option<(Instant, u32)>,
    }
}

impl Skills {
    pub fn knows(&self, skill_id: u32) -> bool {
        self.known.iter().any(|k| k.id == skill_id)
    }

    pub fn level_of(&self, skill_id: u32) -> Option<u8> {
        self.known.iter().find(|k| k.id == skill_id).map(|k| k.level)
    }

    /// What the unit is casting right now.
    pub fn casting_of(&self, obj_id: u32) -> Option<&Casting> {
        self.casting.get(&obj_id)
    }

    /// Time since the player last started `skill_id`.
    pub fn since_used(&self, skill_id: u32) -> Option<Duration> {
        self.last_used.get(&skill_id).map(Instant::elapsed)
    }

    /// Received-only `(remaining, total)` for a skill and its native ordered tag list.
    /// The greatest remaining time wins, tags before direct skill entries; ties keep the first.
    /// This excludes global cooldowns and cast observations. No DB duration replaces wire data.
    pub fn received_cooldown(&self, skill_id: u32, tags: &[u32]) -> Option<(Duration, Duration)> {
        self.received_cooldown_at(skill_id, tags, Instant::now())
    }

    pub fn received_cooldown_at(
        &self,
        skill_id: u32,
        tags: &[u32],
        now: Instant,
    ) -> Option<(Duration, Duration)> {
        let mut result: Option<(Duration, Duration)> = None;
        let candidates = tags
            .iter()
            .flat_map(|tag| {
                self.received_tag_cooldowns
                    .iter()
                    .filter(move |entry| entry.cooldown.id == *tag)
            })
            .chain(
                self.received_skill_cooldowns
                    .iter()
                    .filter(|entry| entry.cooldown.id == skill_id),
            );
        for entry in candidates {
            if let Some(left) = entry.left_at(now) {
                if result.is_none_or(|(best, _)| left > best) {
                    result = Some((
                        left,
                        Duration::from_millis(u64::from(entry.cooldown.total_ms)),
                    ));
                }
            }
        }
        result
    }

    /// `(remaining, total)` from received timers and the existing cast/DB estimate.
    /// The greatest remaining time wins; equal remaining keeps the received candidate and its
    /// native tag/skill tie order. A received total of zero stays zero, never replaced by DB data.
    /// `tag` is the existing single cooldown_tag_id (zero = none).
    pub fn cooldown_with_total(
        &self,
        skill_id: u32,
        tag: u32,
        cooldown_ms: u32,
    ) -> Option<(Duration, Duration)> {
        self.cooldown_with_total_at(skill_id, tag, cooldown_ms, Instant::now())
    }

    fn cooldown_with_total_at(
        &self,
        skill_id: u32,
        tag: u32,
        cooldown_ms: u32,
        now: Instant,
    ) -> Option<(Duration, Duration)> {
        let tags = if tag == 0 {
            &[][..]
        } else {
            std::slice::from_ref(&tag)
        };
        let received = self.received_cooldown_at(skill_id, tags, now);
        let observed = self.last_used.get(&skill_id).and_then(|&used| {
            if tag != 0
                && self
                    .reset_tags
                    .iter()
                    .any(|&(t, at)| t == tag && at >= used)
            {
                return None;
            }
            let total = Duration::from_millis(u64::from(cooldown_ms));
            total
                .checked_sub(now.saturating_duration_since(used))
                .filter(|left| !left.is_zero())
                .map(|left| (left, total))
        });
        match (received, observed) {
            (Some(received), Some(observed)) if observed.0 > received.0 => Some(observed),
            (Some(received), _) => Some(received),
            (None, observed) => observed,
        }
    }

    /// Cooldown left using received timers and the previous cast/DB-duration estimate.
    /// This wraps cooldown_with_total, so slot eligibility and presentation use the same winner.
    pub fn cooldown_left(&self, skill_id: u32, tag: u32, cooldown_ms: u32) -> Option<Duration> {
        self.cooldown_with_total(skill_id, tag, cooldown_ms)
            .map(|(left, _)| left)
    }

    /// Catalog query: the `plot_effects.event_id` of every `GlobalCooldown` special effect with its `value1` (ms).
    /// Static like the cooldown catalog: it survives character and connection resets.
    pub fn set_plot_gcd_catalog(&mut self, catalog: HashMap<u32, u32>) {
        self.plot_gcd_catalog = std::sync::Arc::new(catalog);
    }

    /// (left, total) of the global cooldown an own plot step started, at `now`.
    pub fn plot_gcd_at(&self, now: Instant) -> Option<(Duration, Duration)> {
        let (at, ms) = self.plot_gcd?;
        let total = Duration::from_millis(u64::from(ms));
        let left = total.checked_sub(now.saturating_duration_since(at)).filter(|l| !l.is_zero())?;
        Some((left, total))
    }

    /// Global cooldown left given its length in ms (`custom_gcd` of [`Skills::last_cast_skill`]).
    pub fn gcd_left(&self, gcd_ms: u32) -> Option<Duration> {
        Duration::from_millis(u64::from(gcd_ms)).checked_sub(self.last_cast?.elapsed())
    }

    fn is_own(&self, obj_id: u32) -> bool {
        self.obj_id != 0 && self.obj_id == obj_id
    }

    fn end_cast(&mut self, tl: u16) -> bool {
        let before = self.casting.len();
        self.casting.retain(|_, c| c.tl != tl);
        self.casting.len() != before
    }

    fn learn(&mut self, skill_id: u32, level: u8) {
        match self.known.iter_mut().find(|k| k.id == skill_id) {
            Some(k) => k.level = level,
            None => self.known.push(KnownSkill { id: skill_id, level }),
        }
    }

    pub fn apply(&mut self, e: &Event) -> bool {
        let now = Instant::now();
        match e {
            Event::PlayerSpawn(s) => {
                let last_frame_ms=self.native_last_frame_ms;
                let catalog=std::mem::take(&mut self.cooldown_catalog);
                *self = Self {
                    cooldown_catalog:catalog,
                    native_last_frame_ms:last_frame_ms,
                    native_gm_mode1:None,
                    obj_id: s.obj_id,
                    known: s.unit.skills.iter().map(|&(id, level)| KnownSkill { id, level }).collect(),
                    passive_buffs: s.unit.passive_buffs.clone(),
                    ..Self::default()
                };
            }
            Event::SkillStarted(s) if s.failed() => {
                if !self.is_own(s.caster.obj_id) {
                    return false;
                }
                self.last_failure = Some(SkillFailure { skill_id: s.skill_id, result: s.result, value: s.result_value, at: now });
            }
            Event::SkillStarted(s) => {
                self.casting.insert(
                    s.caster.obj_id,
                    Casting {
                        skill_id: s.skill_id,
                        tl: s.tl,
                        target: s.target,
                        cast_ms: s.cast_ms,
                        base_cast_ms: s.base_cast_ms,
                        started: now,
                        fired: None,
                        channeling_ms: 0,
                    },
                );
                if self.is_own(s.caster.obj_id) {
                    self.last_used.insert(s.skill_id, now);
                    self.last_cast = Some(now);
                    self.last_cast_skill = s.skill_id;
                    self.last_failure = None;
                }
            }
            Event::SkillFired(f) => {
                match self.casting.get_mut(&f.caster.obj_id) {
                    Some(c) if c.tl == f.tl => {
                        c.fired = Some(now);
                        c.channeling_ms = f.channeling_ms;
                    }
                    // No `SkillStarted` came first (the server skips it for instant item skills):
                    // this is where the skill starts.
                    _ => {
                        self.casting.insert(
                            f.caster.obj_id,
                            Casting {
                                skill_id: f.skill_id,
                                tl: f.tl,
                                target: f.target,
                                cast_ms: 0,
                                base_cast_ms: 0,
                                started: now,
                                fired: Some(now),
                                channeling_ms: f.channeling_ms,
                            },
                        );
                        if self.is_own(f.caster.obj_id) {
                            self.last_used.insert(f.skill_id, now);
                            self.last_cast = Some(now);
                            self.last_cast_skill = f.skill_id;
                            self.last_failure = None;
                        }
                    }
                }
            }
            Event::CastingStopped { tl,duration_ms } | Event::PlotStopped {tl,duration_ms,..} => {
                let inserted=self.fold_stop_timer(*tl,*duration_ms,matches!(e,Event::PlotStopped{..}),now);
                return self.end_cast(*tl) || inserted;
            }
            Event::UnitGmModeChanged(change) if self.is_own(change.obj_id) && change.mode==1 => {
                let changed=self.native_gm_mode1!=Some(change.value);
                self.native_gm_mode1=Some(change.value);
                return changed;
            }
            Event::SkillEnded { tl } | Event::PlotEnded { tl } => {
                return self.end_cast(*tl);
            }
            Event::SkillStopped { obj_id, skill_id } => {
                if self.casting.get(obj_id).is_some_and(|c| c.skill_id == *skill_id) {
                    self.casting.remove(obj_id);
                } else {
                    return false;
                }
            }
            Event::PlotEvent(p) => {
                // A step with a `GlobalCooldown` effect restarts the player's global cooldown (the server's
                // `GlobalCooldown = now + value1 * GlobalCooldownMul / 100`; the multiplier is not known here, 100 %).
                let gcd = self.is_own(p.caster_obj_id).then(|| self.plot_gcd_catalog.get(&p.event_id).copied()).flatten();
                if let Some(ms) = gcd {
                    self.plot_gcd = Some((now, ms));
                }
                // The first step of a plot skill starts it (the lane server sends no
                // `SkillStarted` for them); later steps with a cast time show the cast bar.
                let fresh = !self.casting.get(&p.caster_obj_id).is_some_and(|c| c.tl == p.tl);
                if !fresh && p.casting_ms == 0 {
                    return gcd.is_some();
                }
                self.casting.insert(
                    p.caster_obj_id,
                    Casting {
                        skill_id: p.skill_id,
                        tl: p.tl,
                        target: SkillTarget::unit(p.target_obj_id),
                        cast_ms: p.casting_ms,
                        base_cast_ms: p.casting_ms,
                        started: now,
                        fired: None,
                        channeling_ms: 0,
                    },
                );
                if fresh && self.is_own(p.caster_obj_id) {
                    self.last_used.insert(p.skill_id, now);
                    self.last_cast = Some(now);
                    self.last_cast_skill = p.skill_id;
                    self.last_failure = None;
                }
            }
            Event::UnitDeath { obj_id } => {
                if self.casting.remove(obj_id).is_none() {
                    return false;
                }
            }
            Event::UnitsRemoved(ids) => {
                let before = self.casting.len();
                let lost_context = self.obj_id != 0 && ids.contains(&self.obj_id);
                if lost_context {
                    self.native_gm_mode1 = None;
                    self.obj_id = 0;
                }
                for id in ids {
                    self.casting.remove(id);
                }
                return self.casting.len() != before || lost_context;
            }
            Event::Cooldowns(cooldowns) if self.obj_id != 0 => {
                fn append(
                    target: &mut Vec<ReceivedCooldown>,
                    entries: &[Cooldown],
                    now: Instant,
                ) -> bool {
                    let before = target.len();
                    let available = COOLDOWN_LIMIT.saturating_sub(before);
                    target.extend(
                        entries
                            .iter()
                            .take(available)
                            .map(|&cooldown| ReceivedCooldown {
                                cooldown,
                                received_at: now,
                                reset: false,
                            }),
                    );
                    target.len() != before
                }
                let skills = append(&mut self.received_skill_cooldowns, &cooldowns.skills, now);
                let tags = append(&mut self.received_tag_cooldowns, &cooldowns.tags, now);
                let runtime=self.seed_native_timers(cooldowns,now);
                return skills || tags || runtime;
            }
            Event::SkillCooldownReset {
                obj_id,
                skill_id,
                tag_id,
                gcd,
            } if self.is_own(*obj_id) => {
                self.reset_native_timer(*skill_id,*tag_id);
                // Native reset helpers stop at the first matching entry, even if it was reset
                // already. Do not coalesce duplicates or turn this into a list-wide clear.
                if let Some(entry) = self
                    .received_skill_cooldowns
                    .iter_mut()
                    .find(|e| e.cooldown.id == *skill_id)
                {
                    entry.reset = true;
                }
                if let Some(entry) = self
                    .received_tag_cooldowns
                    .iter_mut()
                    .find(|e| e.cooldown.id == *tag_id)
                {
                    entry.reset = true;
                }
                if *gcd {
                    self.last_cast = None;
                    self.last_cast_skill = 0;
                    self.plot_gcd = None;
                }
                if *skill_id != 0 {
                    self.last_used.remove(skill_id);
                }
                if *tag_id != 0 {
                    self.reset_tags.retain(|&(t, _)| t != *tag_id);
                    self.reset_tags.push((*tag_id, now));
                }
            }
            Event::SkillLearned { skill_id, level } | Event::SkillUpgraded { skill_id, level } => {
                self.learn(*skill_id, *level);
            }
            Event::PassiveBuffLearned { obj_id, buff_id } if self.is_own(*obj_id) => {
                if self.passive_buffs.contains(buff_id) {
                    return false;
                }
                self.passive_buffs.push(*buff_id);
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => {
                let previous = self.clone();
                self.clear_received_state();
                return *self != previous;
            }
            _ => return false,
        }
        true
    }
}

impl PartialEq for Skills {
    fn eq(&self, o: &Self) -> bool {
        self.obj_id == o.obj_id
            && self.known == o.known
            && self.passive_buffs == o.passive_buffs
            && self.casting == o.casting
            && self.last_used == o.last_used
            && self.last_cast == o.last_cast
            && self.last_cast_skill == o.last_cast_skill
            && self.reset_tags == o.reset_tags
            && self.received_skill_cooldowns == o.received_skill_cooldowns
            && self.received_tag_cooldowns == o.received_tag_cooldowns
            && self.native_skill_cooldowns == o.native_skill_cooldowns
            && self.native_tag_cooldowns == o.native_tag_cooldowns
            && self.native_gm_mode1 == o.native_gm_mode1
            && self.native_last_frame_ms == o.native_last_frame_ms
            && self.cooldown_catalog == o.cooldown_catalog
            && self.last_failure == o.last_failure
            && self.plot_gcd_catalog == o.plot_gcd_catalog
            && self.plot_gcd == o.plot_gcd
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buff::BuffCaster;
    use crate::skill::{result, PlotEvent, SkillFired, SkillStarted};

    #[test]
    fn removed_units_stop_casting_without_clearing_other_casts() {
        let mut s = Skills { obj_id: 5, ..Skills::default() };
        s.apply(&started(1, 1, 5, 3000, 0));
        s.apply(&started(2, 2, 9, 3000, 0));
        s.apply(&started(3, 3, 10, 3000, 0));
        assert!(s.apply(&Event::UnitsRemoved(vec![9, 11])));
        assert!(s.casting_of(9).is_none());
        assert!(s.casting_of(5).is_some() && s.casting_of(10).is_some());
        assert!(!s.apply(&Event::UnitsRemoved(vec![9, 11])));
    }

    fn started(skill_id: u32, tl: u16, caster: u32, cast_ms: u32, res: u8) -> Event {
        Event::SkillStarted(SkillStarted {
            skill_id,
            tl,
            caster: BuffCaster { kind: 0, obj_id: caster },
            target: SkillTarget::unit(9),
            raw_caster:crate::SkillCasterData::Unit(caster),
            raw_target:crate::SkillTargetData::Unit(9),
            cast_ms,
            base_cast_ms: cast_ms,
            synergy: 0,
            result: res,
            result_value: 0,
            raw_result:crate::SkillResult::default(),
        })
    }

    fn fired(skill_id: u32, tl: u16, caster: u32) -> Event {
        Event::SkillFired(SkillFired {
            skill_id,
            tl,
            caster: BuffCaster { kind: 0, obj_id: caster },
            target: SkillTarget::unit(9),
            raw_caster:crate::SkillCasterData::Unit(caster),
            raw_target:crate::SkillTargetData::Unit(9),
            delay_ms: 0,
            channeling_ms: 0,
            fire_anim_id: 0,
            raw_final_flag: 0,
            result:crate::SkillResult::default(),
        })
    }

    fn skills() -> Skills {
        Skills { obj_id: 5, known: vec![KnownSkill { id: 18132, level: 1 }], ..Skills::default() }
    }

    #[test]
    fn a_cast_runs_from_start_to_end() {
        let mut s = skills();
        assert!(s.apply(&started(18132, 3, 5, 1500, 0)));
        let c = s.casting_of(5).unwrap();
        assert_eq!((c.skill_id, c.tl, c.cast_ms), (18132, 3, 1500));
        assert!(c.progress() < 1.0);
        assert!(s.since_used(18132).is_some() && s.last_cast.is_some());
        assert_eq!(s.last_cast_skill, 18132);
        // Another unit's cast is shown but is not the player's use.
        assert!(s.apply(&started(777, 4, 8, 0, 0)));
        assert!(s.casting_of(8).is_some() && s.since_used(777).is_none());
        assert!(s.apply(&fired(18132, 3, 5)));
        assert_eq!(s.casting_of(5).unwrap().progress(), 1.0);
        assert!(s.apply(&Event::SkillEnded { tl: 3 }));
        assert!(s.casting_of(5).is_none() && s.casting_of(8).is_some());
        assert!(!s.apply(&Event::SkillEnded { tl: 3 }), "nothing left to end");
        assert!(s.apply(&Event::UnitDeath { obj_id: 8 }));
        assert!(s.casting.is_empty());
        // An instant item skill is fired and ended without a start (live on the lane server).
        assert!(s.apply(&fired(25202, 12, 5)));
        assert!(s.since_used(25202).is_some() && s.casting_of(5).is_some());
        assert!(s.apply(&Event::SkillEnded { tl: 12 }));
        assert!(s.casting.is_empty() && s.since_used(25202).is_some());
    }

    #[test]
    fn refused_skills_are_remembered_not_cast() {
        let mut s = skills();
        assert!(s.apply(&started(18132, 0, 5, 0, result::LACK_MANA)));
        assert!(s.casting.is_empty() && s.last_used.is_empty());
        let f = s.last_failure.as_ref().unwrap();
        assert_eq!((f.skill_id, f.result), (18132, result::LACK_MANA));
        // Somebody else's refusal is not ours.
        assert!(!s.apply(&started(1, 0, 99, 0, result::TOO_FAR_RANGE)));
        // A later start clears it.
        assert!(s.apply(&started(18132, 4, 5, 0, 0)));
        assert!(s.last_failure.is_none());
    }

    #[test]
    fn cooldowns_follow_use_and_reset() {
        let mut s = skills();
        assert_eq!(s.cooldown_left(18132, 0, 1000), None);
        s.apply(&started(18132, 3, 5, 0, 0));
        let left = s.cooldown_left(18132, 0, 60_000).unwrap();
        assert!(left > Duration::from_secs(59));
        assert!(s.gcd_left(60_000).is_some());
        assert_eq!(s.cooldown_left(18132, 0, 0), None);
        // A reset of the skill clears its timer.
        assert!(s.apply(&Event::SkillCooldownReset { obj_id: 5, skill_id: 18132, tag_id: 0, gcd: false }));
        assert_eq!(s.cooldown_left(18132, 0, 60_000), None);
        // A reset of a tag makes the skills of that tag ready, but only if used before it.
        s.apply(&started(18132, 4, 5, 0, 0));
        assert!(s.cooldown_left(18132, 77, 60_000).is_some());
        s.apply(&Event::SkillCooldownReset { obj_id: 5, skill_id: 0, tag_id: 77, gcd: true });
        assert_eq!(s.cooldown_left(18132, 77, 60_000), None);
        assert!(s.cooldown_left(18132, 78, 60_000).is_some(), "another tag is unaffected");
        // Not the player's reset.
        assert!(!s.apply(&Event::SkillCooldownReset { obj_id: 6, skill_id: 18132, tag_id: 0, gcd: false }));
    }

    #[test]
    fn learning_and_plots() {
        let mut s = skills();
        assert!(s.knows(18132) && !s.knows(16064));
        assert!(s.apply(&Event::SkillLearned { skill_id: 16064, level: 1 }));
        assert!(s.apply(&Event::SkillUpgraded { skill_id: 16064, level: 3 }));
        assert_eq!((s.level_of(16064), s.known.len()), (Some(3), 2));
        assert!(s.apply(&Event::PassiveBuffLearned { obj_id: 5, buff_id: 9 }));
        assert!(!s.apply(&Event::PassiveBuffLearned { obj_id: 5, buff_id: 9 }));
        assert!(!s.apply(&Event::PassiveBuffLearned { obj_id: 6, buff_id: 10 }));
        // A plot step with a cast time shows a cast bar until the plot ends.
        let plot = Event::PlotEvent(PlotEvent {
            tl: 11,
            event_id: 1,
            skill_id: 16064,
            caster_obj_id: 5,
            target_obj_id: 9,
            caster_pos: None,
            target_pos: None,
            casting_ms: 800,
        });
        assert!(s.apply(&plot));
        assert_eq!(s.casting_of(5).unwrap().cast_ms, 800);
        assert!(s.since_used(16064).is_some(), "the first step starts the skill");
        assert!(s.apply(&Event::PlotEnded { tl: 11 }));
        assert!(s.casting.is_empty());
        // Later steps without a cast time change nothing.
        let step = |tl, casting_ms| {
            Event::PlotEvent(PlotEvent {
                tl,
                event_id: 2,
                skill_id: 16064,
                caster_obj_id: 5,
                target_obj_id: 9,
                caster_pos: None,
                target_pos: None,
                casting_ms,
            })
        };
        assert!(s.apply(&step(12, 0)));
        assert!(!s.apply(&step(12, 0)));
        assert!(s.apply(&step(12, 300)));
        assert_eq!(s.casting_of(5).unwrap().cast_ms, 300);
        assert!(s.apply(&Event::PlotEnded { tl: 12 }));
        assert!(s.apply(&Event::Disconnected { message: String::new() }));
        assert_eq!(s, Skills::default());
    }

    #[test]
    fn a_plot_step_with_a_global_cooldown_effect_starts_it() {
        let mut s = skills();
        s.set_plot_gcd_catalog([(12148, 2000)].into());
        let step = |event_id, caster_obj_id| {
            Event::PlotEvent(PlotEvent { tl: 7, event_id, skill_id: 18132, caster_obj_id, target_obj_id: 9, caster_pos: None, target_pos: None, casting_ms: 0 })
        };
        assert!(s.apply(&step(12129, 5)), "the first step starts the plot");
        assert!(s.plot_gcd.is_none());
        assert!(!s.apply(&step(12130, 5)), "a later step without the effect changes nothing");
        assert!(s.apply(&step(12148, 5)));
        let (left, total) = s.plot_gcd_at(Instant::now()).unwrap();
        assert!(left > Duration::from_millis(1900) && total == Duration::from_secs(2));
        assert_eq!(s.plot_gcd_at(s.plot_gcd.unwrap().0 + Duration::from_secs(2)), None);
        // another unit's step leaves the player's global cooldown alone
        let mut other = skills();
        other.set_plot_gcd_catalog([(12148, 2000)].into());
        assert!(other.apply(&step(12148, 6)) && other.plot_gcd.is_none());
        // a reset of the global cooldown clears it; the catalog survives a disconnect
        assert!(s.apply(&Event::SkillCooldownReset { obj_id: 5, skill_id: 0, tag_id: 0, gcd: true }));
        assert!(s.plot_gcd.is_none());
        s.apply(&Event::Disconnected { message: String::new() });
        assert_eq!(s.plot_gcd_catalog.get(&12148), Some(&2000));
    }
    fn received(id: u32, total_ms: u32, remaining_ms: u32, at: Instant) -> ReceivedCooldown {
        ReceivedCooldown {
            cooldown: Cooldown {
                id,
                total_ms,
                remaining_ms,
            },
            received_at: at,
            reset: false,
        }
    }

    #[test]
    fn received_timers_are_independent_of_casts_and_select_native_maximum() {
        let now = Instant::now();
        let mut s = skills();
        s.received_skill_cooldowns = vec![
            received(18132, 20_000, 5000, now),
            received(18132, 30_000, 6000, now),
        ];
        s.received_tag_cooldowns = vec![
            received(77, 10_000, 6000, now),
            received(78, 40_000, 6000, now),
        ];
        let ms = Duration::from_millis;
        assert_eq!(
            s.received_cooldown_at(18132, &[77, 78], now),
            Some((ms(6000), ms(10_000))),
            "tags before skills and ordered ties"
        );
        assert_eq!(
            s.received_cooldown_at(18132, &[78, 77], now),
            Some((ms(6000), ms(40_000)))
        );
        assert_eq!(
            s.received_cooldown_at(18132, &[], now + ms(2000)),
            Some((ms(4000), ms(30_000)))
        );
        assert_eq!(s.received_cooldown_at(18132, &[77], now + ms(6000)), None);
        assert_eq!(
            s.received_cooldown_at(999, &[77], now),
            Some((ms(6000), ms(10_000))),
            "tag does not require a cast or learned skill"
        );
        assert!(s.last_used.is_empty() && s.last_cast.is_none() && !s.knows(999));
        assert!(
            s.cooldown_left(18132, 0, 0).unwrap() > ms(5000),
            "received timer needs no DB duration"
        );
        s.last_used.insert(18132, now);
        assert!(
            s.cooldown_left(18132, 77, 60_000).unwrap() > ms(59_000),
            "retain existing cast estimate when it is greater"
        );
        assert_eq!(
            s.received_skill_cooldowns[0].cooldown.remaining_ms, 5000,
            "queries never rewrite wire data"
        );
        let zero = received(0, 0, 0, now);
        assert_eq!(zero.left_at(now), None);
        let raw = received(0, 1, u32::MAX, now);
        assert_eq!(
            raw.left_at(now),
            Some(ms(u32::MAX as u64)),
            "native does not clamp remaining to total"
        );
    }

    #[test]
    fn received_lists_append_until_native_capacity_and_empty_never_clears() {
        use crate::skill::SkillCooldowns;
        let packet = Event::Cooldowns(SkillCooldowns {
            skills: vec![
                Cooldown {
                    id: 1,
                    total_ms: 1000,
                    remaining_ms: 999
                };
                149
            ],
            tags: vec![],
        });
        let mut s = Skills::default();
        assert!(
            !s.apply(&packet),
            "no local player: no fabricated ownership"
        );
        s.obj_id = 5;
        assert!(s.apply(&packet));
        assert!(s.apply(&packet));
        assert_eq!(s.received_skill_cooldowns.len(), COOLDOWN_LIMIT);
        let before = s.clone();
        assert!(
            !s.apply(&packet),
            "full native list ignores additional records"
        );
        assert!(!s.apply(&Event::Cooldowns(SkillCooldowns::default())));
        assert_eq!(s, before);
        assert!(s.apply(&Event::Cooldowns(SkillCooldowns {
            skills: vec![],
            tags: vec![Cooldown {
                id: 2,
                total_ms: 2000,
                remaining_ms: 1000
            }]
        })));
        assert_eq!(
            s.received_tag_cooldowns.len(),
            1,
            "tag capacity is independent"
        );
    }

    #[test]
    fn received_resets_clear_first_match_only_and_preserve_original_wire_fields() {
        let now = Instant::now();
        let mut s = skills();
        s.received_skill_cooldowns = vec![
            received(18132, 2000, 1000, now),
            received(18132, 3000, 2000, now),
        ];
        s.received_tag_cooldowns = vec![received(77, 2000, 1000, now)];
        s.apply(&started(18132, 4, 5, 0, 0));
        let reset = |obj_id, gcd| Event::SkillCooldownReset {
            obj_id,
            skill_id: 18132,
            tag_id: 77,
            gcd,
        };
        assert!(!s.apply(&reset(6, true)));
        assert!(s.last_cast.is_some());
        assert!(s.apply(&reset(5, true)));
        assert!(s.last_cast.is_none() && s.last_cast_skill == 0);
        assert!(s.received_skill_cooldowns[0].reset && !s.received_skill_cooldowns[1].reset);
        assert!(s.received_tag_cooldowns[0].reset);
        assert_eq!(s.received_skill_cooldowns[0].cooldown.remaining_ms, 1000);
        assert_eq!(
            s.received_cooldown_at(18132, &[77], now),
            Some((Duration::from_millis(2000), Duration::from_millis(3000)))
        );
        s.apply(&reset(5, false));
        assert!(
            !s.received_skill_cooldowns[1].reset,
            "native repeated reset still stops at the first match"
        );
        assert_eq!(s.since_used(18132), None);
    }

    #[test]
    fn received_cooldowns_clear_on_character_spawn_selection_leave_and_disconnect() {
        let now = Instant::now();
        let mut original = skills();
        original
            .received_skill_cooldowns
            .push(received(18132, 1000, 1000, now));
        original
            .received_tag_cooldowns
            .push(received(77, 2000, 2000, now));
        for event in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 0 },
            Event::Disconnected {
                message: String::new(),
            },
            Event::PlayerSpawn(Box::new(crate::session::PlayerSpawn {
                obj_id: 6,
                state: Default::default(),
                unit: crate::state::tests::unit(6),
            })),
        ] {
            let mut s = original.clone();
            assert!(s.apply(&event));
            assert!(s.received_skill_cooldowns.is_empty() && s.received_tag_cooldowns.is_empty());
            assert_eq!(s.received_cooldown_at(18132, &[77], now), None);
        }
    }

    #[test]
    fn combined_cooldown_uses_one_clock_and_returns_winning_total_without_fallback() {
        let now = Instant::now();
        let ms = Duration::from_millis;
        let mut s = skills();
        s.last_used.insert(18132, now);
        s.received_skill_cooldowns
            .push(received(18132, 10_000, 5000, now));
        s.received_tag_cooldowns.push(received(77, 0, 5000, now));
        assert_eq!(
            s.cooldown_with_total_at(18132, 77, 5000, now),
            Some((ms(5000), ms(0))),
            "tie keeps native tag winner including zero total"
        );
        assert_eq!(
            s.cooldown_with_total_at(18132, 77, 6000, now),
            Some((ms(6000), ms(6000))),
            "greater observed remaining uses DB total"
        );
        assert_eq!(
            s.cooldown_with_total_at(18132, 0, 4000, now + ms(1000)),
            Some((ms(4000), ms(10_000))),
            "greater received remaining uses actual received total"
        );
        s.reset_tags.push((77, now));
        assert_eq!(
            s.cooldown_with_total_at(18132, 77, 60_000, now),
            Some((ms(5000), ms(0))),
            "tag reset excludes use estimate but not unrelated received direct entries"
        );
        s.received_skill_cooldowns[0].reset = true;
        s.received_tag_cooldowns[0].reset = true;
        assert_eq!(s.cooldown_with_total_at(18132, 77, 60_000, now), None);
        assert_eq!(s.cooldown_with_total(18132, 77, 60_000), None);
        assert_eq!(s.cooldown_left(18132, 77, 60_000), None);
    }

}
