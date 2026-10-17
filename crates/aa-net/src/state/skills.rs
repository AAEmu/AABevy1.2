//! Skills: what the player knows, what units are casting, and when skills were last used.
//!
//! The server never sends cooldown lengths or timers: the client keeps the time of the last use
//! of each skill and takes the cooldown (`skills.cooldown_time`, `custom_gcd`, `default_gcd`
//! in the game database) from its tables. A `SkillCooldownReset` clears the timer.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::session::Event;
use crate::skill::SkillTarget;

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
        /// The last refused skill (not enough mana, too far ...).
        pub last_failure: Option<SkillFailure>,
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

    /// Cooldown left of `skill_id` given its cooldown time in ms (`skills.cooldown_time`); `None`
    /// when it is ready. `tag` is the skill's `cooldown_tag_id` (0 for none): a reset of the tag
    /// after the last use makes the skill ready.
    pub fn cooldown_left(&self, skill_id: u32, tag: u32, cooldown_ms: u32) -> Option<Duration> {
        let used = *self.last_used.get(&skill_id)?;
        if tag != 0 && self.reset_tags.iter().any(|&(t, at)| t == tag && at >= used) {
            return None;
        }
        Duration::from_millis(u64::from(cooldown_ms)).checked_sub(used.elapsed())
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
                *self = Self {
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
            Event::SkillEnded { tl } | Event::CastingStopped { tl, .. } | Event::PlotEnded { tl } => {
                return self.end_cast(*tl);
            }
            Event::PlotStopped { tl, .. } => return self.end_cast(*tl),
            Event::SkillStopped { obj_id, skill_id } => {
                if self.casting.get(obj_id).is_some_and(|c| c.skill_id == *skill_id) {
                    self.casting.remove(obj_id);
                } else {
                    return false;
                }
            }
            Event::PlotEvent(p) => {
                // The first step of a plot skill starts it (the lane server sends no
                // `SkillStarted` for them); later steps with a cast time show the cast bar.
                let fresh = !self.casting.get(&p.caster_obj_id).is_some_and(|c| c.tl == p.tl);
                if !fresh && p.casting_ms == 0 {
                    return false;
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
            Event::SkillCooldownReset { obj_id, skill_id, tag_id, .. } if self.is_own(*obj_id) => {
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
            Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if *self == Self::default() {
                    return false;
                }
                *self = Self::default();
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
            && self.last_failure == o.last_failure
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buff::BuffCaster;
    use crate::skill::{result, PlotEvent, SkillFired, SkillStarted};

    fn started(skill_id: u32, tl: u16, caster: u32, cast_ms: u32, res: u8) -> Event {
        Event::SkillStarted(SkillStarted {
            skill_id,
            tl,
            caster: BuffCaster { kind: 0, obj_id: caster },
            target: SkillTarget::unit(9),
            cast_ms,
            base_cast_ms: cast_ms,
            synergy: 0,
            result: res,
            result_value: 0,
        })
    }

    fn fired(skill_id: u32, tl: u16, caster: u32) -> Event {
        Event::SkillFired(SkillFired {
            skill_id,
            tl,
            caster: BuffCaster { kind: 0, obj_id: caster },
            target: SkillTarget::unit(9),
            delay_ms: 0,
            channeling_ms: 0,
            fire_anim_id: 0,
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
}
