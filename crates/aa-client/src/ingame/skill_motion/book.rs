//! What the game database says about skills that move units: the controllers a plot step or a skill starts, and the `KnockBack`
//! special effects. Looked up once per id and cached.
//!
//! * `plot_effects` (`event_id`, `source_id`, `target_id`, `actual_type`, `actual_id`): `SkillController` rows start a controller on
//!   the unit `source_id` names, with the unit `target_id` names as its reference; `SpecialEffect` rows of type 13 are knockbacks
//!   of `target_id` away from `source_id`. The ids are the plot's: source 1 the skill's caster, 2 its target, 3 the step's source
//!   (the `SCPlotEvent` caster), 4 the step's target; target 5 is the step's target too.
//! * `skills.skill_controller_id` is the controller of a skill without a plot (Drop Back); a plot skill moves by its steps.
//! * `skill_effects` of type `SpecialEffect` 13: the knockbacks of a plain skill, applied when its damage lands.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OpenFlags, params, types::Value};

use super::path::Controller;

/// The `KnockBack` special effect: `value1` mm (0: 3 m) away from the source, launched at `value2` degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knock {
    pub distance: f32,
    pub angle: f32,
}

/// A blink of the caster (the server answers it with `SCBlinkUnit`): `Blink` goes `value1` metres forward, `TeleportToUnit` to behind the
/// target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Blink {
    Forward(f32),
    Behind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Controller(Controller),
    Knock(Knock),
}

/// One movement effect of a plot step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepMove {
    pub source: u8,
    pub target: u8,
    pub effect: Effect,
}

/// What a skill itself moves (a plot skill has no controller of its own: its steps move).
#[derive(Clone, Debug, Default)]
pub struct SkillMoves {
    /// The controller of a skill without a plot, with its clip.
    pub controller: Option<Controller>,
    /// Knockbacks that apply to each unit the skill hurts.
    pub knocks: Vec<Knock>,
    /// The blink of the caster, if the skill has one.
    pub blink: Option<Blink>,
}

#[derive(bevy::prelude::Resource)]
pub struct Book(Mutex<Inner>);

struct Inner {
    conn: Connection,
    controllers: HashMap<u32, Option<Controller>>,
    steps: HashMap<u32, Arc<Vec<StepMove>>>,
    skills: HashMap<u32, Arc<SkillMoves>>,
    /// `buffs` that stun, put to sleep or root (loaded on first use): a unit under one only starts Leap controllers.
    rooting: Option<HashSet<u32>>,
}

/// `special_effects.special_effect_type_id` of `KnockBack`, `Blink` and `TeleportToUnit`.
const KNOCK_BACK: i64 = 13;
const BLINK: i64 = 11;
const TELEPORT_TO_UNIT: i64 = 47;

fn as_id(v: Value) -> Option<u32> {
    match v {
        Value::Integer(n) if n > 0 => Some(n as u32),
        _ => None,
    }
}

impl Book {
    pub fn open() -> Option<Book> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI).ok()?;
        Some(Book(Mutex::new(Inner { conn, controllers: HashMap::new(), steps: HashMap::new(), skills: HashMap::new(), rooting: None })))
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The movement effects of a plot step.
    pub fn step(&self, event_id: u32) -> Arc<Vec<StepMove>> {
        self.inner().step(event_id)
    }

    pub fn skill(&self, id: u32) -> Arc<SkillMoves> {
        self.inner().skill(id)
    }

    /// The controller a buff starts on its owner when it arrives (`buffs.skill_controller_id`: Mana Force's four variants push their
    /// target), if the row exists (a missing row is no controller, `docs/research/re-skills.md` section 1.1).
    pub fn buff_controller(&self, id: u32) -> Option<Controller> {
        let mut inner = self.inner();
        let controller: i64 = inner.conn.query_row("SELECT skill_controller_id FROM buffs WHERE id = ?1", params![id], |r| r.get::<_, Option<i64>>(0).map(|v| v.unwrap_or(0))).ok()?;
        (controller > 0).then(|| inner.controller(controller as u32)).flatten().filter(|c| matches!(c.kind, 2 | 4))
    }

    /// Whether buff `id` stuns, puts to sleep or roots its owner (`Unit +0x3A70..72`: movement speed is 0, and a controller other than a
    /// Leap does not start).
    pub fn is_rooting(&self, id: u32) -> bool {
        let mut inner = self.inner();
        let Inner { conn, rooting, .. } = &mut *inner;
        rooting
            .get_or_insert_with(|| {
                conn.prepare("SELECT id FROM buffs WHERE stun = 't' OR sleep = 't' OR root = 't'")
                    .and_then(|mut q| q.query_map([], |r| r.get::<_, i64>(0).map(|id| id as u32))?.collect())
                    .unwrap_or_default()
            })
            .contains(&id)
    }
}

impl Inner {
    fn controller(&mut self, id: u32) -> Option<Controller> {
        if let Some(c) = self.controllers.get(&id) {
            return *c;
        }
        let conn = &self.conn;
        let c = conn
            .query_row(
                "SELECT kind_id, value1, value2, value3, value4, value5, value6, value7, value8, value9, value10, value11, value12, value13, start_anim_id \
                 FROM skill_controllers WHERE id = ?1",
                params![id],
                |r| {
                    let int = |i: usize| match r.get::<_, Value>(i) {
                        Ok(Value::Integer(n)) => n as i32,
                        Ok(Value::Real(f)) => f as i32,
                        _ => 0,
                    };
                    let mut v = [0; 13];
                    for (i, slot) in v.iter_mut().enumerate() {
                        *slot = int(i + 1);
                    }
                    Ok(Controller { id, kind: int(0) as u32, v, start_anim: int(14) as u32 })
                },
            )
            .ok();
        self.controllers.insert(id, c);
        c
    }

    fn knock_of(&self, special_id: u32) -> Option<Knock> {
        let (kind, v1, v2): (i64, i64, i64) = self
            .conn
            .query_row("SELECT special_effect_type_id, value1, value2 FROM special_effects WHERE id = ?1", params![special_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .ok()?;
        (kind == KNOCK_BACK).then(|| Knock { distance: if v1 > 0 { v1 as f32 * 0.001 } else { 3.0 }, angle: v2 as f32 })
    }

    fn blink_of(&self, special_id: u32) -> Option<Blink> {
        let (kind, v1): (i64, i64) = self
            .conn
            .query_row("SELECT special_effect_type_id, value1 FROM special_effects WHERE id = ?1", params![special_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .ok()?;
        match kind {
            BLINK => Some(Blink::Forward(v1 as f32)),
            TELEPORT_TO_UNIT => Some(Blink::Behind),
            _ => None,
        }
    }

    fn step(&mut self, event_id: u32) -> Arc<Vec<StepMove>> {
        if let Some(s) = self.steps.get(&event_id) {
            return s.clone();
        }
        let rows: Vec<(i64, i64, String, u32)> = self
            .conn
            .prepare("SELECT source_id, target_id, actual_type, actual_id FROM plot_effects WHERE event_id = ?1 ORDER BY position")
            .and_then(|mut q| q.query_map(params![event_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect())
            .unwrap_or_default();
        let mut moves = Vec::new();
        for (source, target, kind, id) in rows {
            let effect = match kind.as_str() {
                "SkillController" => self.controller(id).filter(|c| matches!(c.kind, 2 | 4)).map(Effect::Controller),
                "SpecialEffect" => self.knock_of(id).map(Effect::Knock),
                _ => None,
            };
            if let Some(effect) = effect {
                moves.push(StepMove { source: source as u8, target: target as u8, effect });
            }
        }
        let moves = Arc::new(moves);
        self.steps.insert(event_id, moves.clone());
        moves
    }

    fn skill(&mut self, id: u32) -> Arc<SkillMoves> {
        if let Some(s) = self.skills.get(&id) {
            return s.clone();
        }
        let head: Option<(Value, Value)> = self.conn.query_row("SELECT plot_id, skill_controller_id FROM skills WHERE id = ?1", params![id], |r| Ok((r.get(0)?, r.get(1)?))).ok();
        let (plot, controller) = head.map_or((false, None), |(plot, sc)| (as_id(plot).is_some(), as_id(sc)));
        let controller = if plot { None } else { controller.and_then(|c| self.controller(c)).filter(|c| matches!(c.kind, 2 | 4)) };
        let effects: Vec<u32> = self
            .conn
            .prepare(
                "SELECT e.actual_id FROM skill_effects se JOIN effects e ON e.id = se.effect_id \
                 WHERE se.skill_id = ?1 AND e.actual_type = 'SpecialEffect'",
            )
            .and_then(|mut q| q.query_map(params![id], |r| r.get(0))?.collect())
            .unwrap_or_default();
        let blink = effects.iter().find_map(|&e| self.blink_of(e));
        let knocks = effects.into_iter().filter_map(|e| self.knock_of(e)).collect();
        let skill = Arc::new(SkillMoves { controller, knocks, blink });
        self.skills.insert(id, skill.clone());
        skill
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Option<Book> {
        Book::open()
    }

    #[test]
    fn charge_moves_its_caster_by_the_launch_step() {
        let Some(book) = book() else { return };
        // Charge 11918: plot step 5100 ("launch") starts controller 6779 on source 1 (the caster) towards target 4 (the step's target)
        let step = book.step(5100);
        let [m] = step.as_slice() else { panic!("{step:?}") };
        assert_eq!((m.source, m.target), (1, 4));
        let Effect::Controller(c) = m.effect else { panic!() };
        assert_eq!((c.id, c.kind, c.v[1], c.v[2], c.v[3]), (6779, 2, 10, 400, -800));
        // a plot skill has no controller of its own to start
        assert!(book.skill(11918).controller.is_none());
    }

    #[test]
    fn a_missing_controller_row_is_no_controller() {
        let Some(book) = book() else { return };
        // Bull Rush (10501) names controller 4433, which has no row, and the real client then moves nobody: the shove is the target's
        // knockdown pose (`docs/research/re-skills.md` section 1.4). Precision Strike (controller 703) and Triple Slash (604) the same.
        assert!(book.step(2225).iter().all(|m| !matches!(m.effect, Effect::Controller(_))));
        for skill in [10501, 12026, 18132] {
            assert!(book.skill(skill).controller.is_none(), "{skill}");
        }
    }

    #[test]
    fn lasso_moves_the_target_to_the_caster() {
        let Some(book) = book() else { return };
        let step = book.step(2659);
        let m = step.iter().find(|m| matches!(m.effect, Effect::Controller(_))).expect("lasso pull");
        // source 4 (the step's target) goes towards target 3 (the step's source, the caster)
        assert_eq!((m.source, m.target), (4, 3));
    }

    #[test]
    fn drop_back_is_a_skill_controller_and_overwhelm_knocks() {
        let Some(book) = book() else { return };
        let drop_back = book.skill(12049);
        assert_eq!(drop_back.controller.map(|c| (c.id, c.v[0], c.v[3])), Some((8074, 15, -10000)));
        // Focal Concussion (11353) pushes 0.4 m
        let knocks = &book.skill(11353).knocks;
        assert!(knocks.iter().any(|k| (k.distance - 0.4).abs() < 1e-4), "{knocks:?}");
        // Teleportation blinks 15 m forward, Shadow Step to behind its target
        assert_eq!(book.skill(10152).blink, Some(Blink::Forward(15.0)));
        assert_eq!(book.skill(12075).blink, Some(Blink::Behind));
    }
}
