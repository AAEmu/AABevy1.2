//! Dev: plays skills one after another on the local player and takes frames of each, for the animation audit
//! (`docs/research/skill-audit-anim.md`).
//!
//! `AA_COMBAT_SWEEP=10377,16064,...` lists the skills; `AA_COMBAT_SWEEP_DIR=shots/skill-audit/anim/frames` is where the frames go
//! (`<id>_<n>.png`, named by the order they were taken in). The events are the ones the server would send (`AA_COMBAT_FAKE` does the
//! same by hand): a skill with a cast time starts, then fires after it; a plot plays its steps (the ones with a clip, effect or projectile) 0.9 s
//! apart; the blow lands after the flight; a channel ends after its time. The caster and the target are the player. Frames come 0.1,
//! 0.4, 0.8 and 1.4 s after the fire (or each of the first four steps), and one halfway through a cast time.
//! `AA_COMBAT_SWEEP_WEAPON=two` / `dual` skips the plot steps of the other weapon classes' clips (`AA_DEV_MAINHAND=5660` makes the player a two
//! handed fighter): the one handed ones are played by default.

use aa_net::Pos;
use aa_net::skill::{CastAction, PlotEvent, SkillFired, SkillStarted, SkillTarget, UnitDamage, target_kind};

use super::super::player::Mover;

use super::*;

/// Seconds after the first frame the sweep starts (the world has to stream in).
const WARM_UP: f64 = 14.0;
/// Seconds between the steps of a plot.
const STEP_GAP: f64 = 0.9;
/// Seconds between the end of a skill and the start of the next.
const PAUSE: f64 = 1.2;

#[derive(Resource)]
pub(super) struct Sweep {
    /// What is left to play, last first: (is a buff, id).
    queue: Vec<(bool, u32)>,
    dir: String,
    weapon: String,
    started: Option<f64>,
    /// What is due, latest first: (time on the app clock, what).
    script: Vec<(f64, Due)>,
    /// When the script of the skill that plays now is over.
    free_at: f64,
    taken: u32,
    last_shot: f64,
    index: u16,
}

enum Due {
    Event(Event),
    /// (is a buff, id, number): the frame `<b><id>_<number>.png`.
    Shot(bool, u32, u32),
}

impl Sweep {
    pub(super) fn from_env() -> Option<Sweep> {
        // `b509` is a buff put on the player, a plain number a skill
        let ids: Vec<(bool, u32)> = std::env::var("AA_COMBAT_SWEEP").ok()?.split(',').filter_map(|s| match s.trim().strip_prefix('b') {
            Some(b) => b.parse().ok().map(|b| (true, b)),
            None => s.trim().parse().ok().map(|s| (false, s)),
        }).collect();
        (!ids.is_empty()).then(|| Sweep {
            queue: ids.into_iter().rev().collect(),
            dir: std::env::var("AA_COMBAT_SWEEP_DIR").unwrap_or_else(|_| "shots/skill-audit/anim/frames".into()),
            weapon: std::env::var("AA_COMBAT_SWEEP_WEAPON").unwrap_or_default(),
            started: None,
            script: Vec::new(),
            free_at: 0.0,
            taken: 0,
            last_shot: 0.0,
            index: 0,
        })
    }

    /// The steps of a plot that this weapon class plays and that show something.
    fn steps(&self, book: &SkillBook, skill: u32) -> Vec<u32> {
        let other = |clip: &str| match self.weapon.as_str() {
            "two" => clip.starts_with("twoswords_") || clip.starts_with("onehand_"),
            "dual" => clip.starts_with("twohand_") || clip.starts_with("onehand_"),
            _ => clip.starts_with("twoswords_") || clip.starts_with("twohand_"),
        };
        book.plot_steps(skill)
            .into_iter()
            .filter(|&id| book.plot_event(id).is_some_and(|p| !p.clip().is_some_and(|c| other(c)) && !p.stops_only()))
            .take(12)
            .collect()
    }

    /// Queues a buff put on the player: its aura and pose, frames while it lasts and after it went; returns when it is over.
    fn plan_buff(&mut self, book: &SkillBook, id: u32, me: u32, t0: f64) -> f64 {
        self.index += 1;
        let index = 7000 + u32::from(self.index);
        let caster = BuffCaster { kind: 0, obj_id: me };
        let mut script: Vec<(f64, Due)> = vec![(t0, Due::Event(Event::BuffCreated { obj_id: me, buff: aa_net::buff::Buff { index, buff_id: id, duration_ms: 0, caster, stacks: 1, ..Default::default() } }))];
        for (n, dt) in [0.5, 1.3, 2.2].into_iter().enumerate() {
            script.push((t0 + dt, Due::Shot(true, id, n as u32)));
        }
        script.push((t0 + 2.5, Due::Event(Event::BuffRemoved { obj_id: me, index })));
        script.push((t0 + 2.8, Due::Shot(true, id, 3)));
        info!("sweep: buff {id}: {:?}", book.buff(id).map(|b| (b.fx.len(), b.start_anim.clone(), b.framehold, b.ragdoll)));
        self.script.extend(script);
        self.script.sort_by(|a, b| b.0.total_cmp(&a.0));
        t0 + 3.6
    }

    /// Queues the events and shots of one skill, starting at `t0`; returns when it is over.
    fn plan(&mut self, book: &SkillBook, id: u32, me: u32, other: Option<u32>, point: Pos, t0: f64) -> f64 {
        let Some(v) = book.skill(id) else {
            warn!("sweep: skill {id} is not in the database");
            return t0;
        };
        self.index += 1;
        let tl = 3000 + self.index;
        let caster = BuffCaster { kind: 0, obj_id: me };
        // a unit to hit, else a spot a few metres ahead of the player (flights are seen, nothing is hit)
        let target = match other {
            Some(o) => SkillTarget { kind: target_kind::UNIT, obj_id: o, pos: None },
            None => SkillTarget { kind: target_kind::POSITION, obj_id: 0, pos: Some(point) },
        };
        let (raw_caster, raw_target) = (aa_net::SkillCasterData::Unit(me), aa_net::SkillTargetData::Unit(other.unwrap_or(0)));
        let other = other.unwrap_or(0);
        let mut script: Vec<(f64, Due)> = Vec::new();
        let shot = |script: &mut Vec<(f64, Due)>, at: f64, n: u32| script.push((at, Due::Shot(false, id, n)));
        let mut end = t0;
        if v.plot {
            let steps = self.steps(book, id);
            for (k, step) in steps.iter().enumerate() {
                let at = t0 + k as f64 * STEP_GAP;
                script.push((at, Due::Event(Event::PlotEvent(PlotEvent { tl, event_id: *step, skill_id: id, caster_obj_id: me, target_obj_id: other, caster_pos: None, target_pos: (other == 0).then_some(point), casting_ms: 0 }))));
                if k < 4 {
                    shot(&mut script, at + 0.35, k as u32);
                }
                // the blow of the step lands as it plays
                if other != 0 {
                script.push((at + 0.3, Due::Event(Event::UnitDamaged(UnitDamage { action: CastAction { kind: 0, skill_id: id, tl, buff_id: 0 }, caster, caster_obj_id: me, target_obj_id: other, damage: 50, absorbed: 0, mana_burn: 0, hit_type: 0, holdable_id: 0, raw_hit: 0x0120 }))));
                }
                end = at + STEP_GAP;
            }
            if steps.is_empty() {
                shot(&mut script, t0 + 0.3, 0);
            }
            script.push((end + 0.2, Due::Event(Event::PlotEnded { tl })));
        } else {
            let cast = f64::from(v.cast_ms) / 1000.0;
            let fire = t0 + cast;
            if v.cast_ms > 0 {
                script.push((t0, Due::Event(Event::SkillStarted(SkillStarted { skill_id: id, tl, caster, target, cast_ms: v.cast_ms, base_cast_ms: v.cast_ms, synergy: 0, result: 0, result_value: 0, raw_result: Default::default(), raw_caster, raw_target }))));
                shot(&mut script, t0 + cast * 0.5, 4);
            }
            // a flying skill lands after the flight
            let delay = if v.projectile.is_some() { v.effect_delay_ms as i32 + 300 } else { v.effect_delay_ms as i32 };
            script.push((fire, Due::Event(Event::SkillFired(SkillFired { skill_id: id, tl, caster, target, delay_ms: delay, channeling_ms: v.channel_ms as i32, fire_anim_id: 0, raw_final_flag: 0, result: Default::default(), raw_caster, raw_target }))));
            for (n, dt) in [0.1, 0.4, 0.8, 1.4].into_iter().enumerate() {
                shot(&mut script, fire + dt, n as u32);
            }
            if other != 0 {
            script.push((fire + 0.002 + f64::from(delay) / 1000.0, Due::Event(Event::UnitDamaged(UnitDamage { action: CastAction { kind: 0, skill_id: id, tl, buff_id: 0 }, caster, caster_obj_id: me, target_obj_id: other, damage: 50, absorbed: 0, mana_burn: 0, hit_type: 0, holdable_id: 0, raw_hit: 0x0120 }))));
            }
            end = fire + 1.6;
            if v.channel_ms > 0 {
                end = end.max(fire + f64::from(v.channel_ms) / 1000.0 + 0.3);
                script.push((fire + f64::from(v.channel_ms) / 1000.0, Due::Event(Event::SkillEnded { tl })));
            }
        }
        // the good buffs of the skill land on the caster (debuffs have their own sweep: `b<id>`)
        let first = script.iter().map(|(t, _)| *t).fold(f64::MAX, f64::min);
        for buff in book.skill_buffs(id) {
            if book.buff(buff).is_some_and(|l| !l.bad) {
                self.index += 1;
                let index = 5000 + u32::from(self.index);
                let created = Event::BuffCreated { obj_id: me, buff: aa_net::buff::Buff { index, buff_id: buff, duration_ms: 0, caster, stacks: 1, ..Default::default() } };
                script.push((first + cast_gap(v.cast_ms), Due::Event(created)));
                script.push((end + 0.1, Due::Event(Event::BuffRemoved { obj_id: me, index })));
            }
        }
        end = end.max(script.iter().map(|(t, _)| *t).fold(t0, f64::max) + 0.5);
        info!("sweep: skill {id}: {} events, {} frames, over at +{:.1} s", script.iter().filter(|(_, d)| matches!(d, Due::Event(_))).count(), script.iter().filter(|(_, d)| matches!(d, Due::Shot(..))).count(), end - t0);
        self.script.extend(script);
        self.script.sort_by(|a, b| b.0.total_cmp(&a.0));
        end
    }
}

/// Seconds from the first event of a skill to its fire (the buffs land with the fire).
fn cast_gap(cast_ms: u32) -> f64 {
    f64::from(cast_ms) / 1000.0 + 0.15
}

/// Plays the sweep: events and screenshots when they are due, the next skill when the one before is over.
pub(super) fn run_sweep(
    mut commands: Commands,
    time: Res<Time>,
    real: Res<Time<Real>>,
    book: Res<SkillBook>,
    local: Query<(&LocalPlayer, &GlobalTransform, &Mover)>,
    units: Res<Units>,
    drawn: Query<&RemoteUnit>,
    mut sweep: ResMut<Sweep>,
    mut buffs: Option<ResMut<aa_net::state::Buffs>>,
    mut out: MessageWriter<NetEvent>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok((me, me_at, mover)) = local.single() else { return };
    let now = time.elapsed_secs_f64();
    let started = *sweep.started.get_or_insert(now);
    if now - started < WARM_UP {
        return;
    }
    while sweep.script.last().is_some_and(|(t, _)| *t <= now) {
        let (_, due) = sweep.script.pop().unwrap();
        match due {
            Due::Event(event) => {
                // the buff list is the net state's: events written here do not reach it
                if let (Some(buffs), Event::BuffCreated { .. } | Event::BuffRemoved { .. }) = (buffs.as_mut(), &event) {
                    buffs.apply(&event);
                }
                out.write(NetEvent(event));
            }
            Due::Shot(buff, id, n) => {
                let file = format!("{}/{}{id}_{n}.png", sweep.dir, if buff { "b" } else { "" });
                if let Some(dir) = std::path::Path::new(&file).parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                commands.spawn(bevy::render::view::screenshot::Screenshot::primary_window()).observe(bevy::render::view::screenshot::save_to_disk(file));
                sweep.taken += 1;
                sweep.last_shot = real.elapsed_secs_f64();
            }
        }
    }
    if now >= sweep.free_at && sweep.script.is_empty() {
        match sweep.queue.pop() {
            Some((true, id)) => {
                let free = sweep.plan_buff(&book, id, me.obj_id, now + PAUSE);
                sweep.free_at = free;
            }
            Some((false, id)) => {
                // the target: the nearest unit that is alive and 2.5 to 20 m away (a rat of the pond), else the player itself
                let here = me_at.translation();
                let near = units
                    .by_id
                    .values()
                    .filter(|u| u.obj_id != me.obj_id && !u.dead && drawn.iter().any(|r| r.obj_id == u.obj_id))
                    .map(|u| (cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z)).distance(here), u.obj_id))
                    .filter(|(d, _)| (2.5..=20.0).contains(d))
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let mut all: Vec<(f32, u32, &str)> = units.by_id.values().filter(|u| u.obj_id != me.obj_id).map(|u| (cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z)).distance(here), u.obj_id, u.name.as_str())).collect();
                all.sort_by(|a, b| a.0.total_cmp(&b.0));
                info!("sweep: nearest units {:?}", all.iter().take(4).map(|(d, id, n)| format!("{n} {id} {d:.1} m")).collect::<Vec<_>>());
                let other = near.map(|(_, id)| id);
                // six metres ahead (game coordinates: facing 0 is north, counter-clockwise)
                let point = Pos::new(mover.pos.x - mover.facing.sin() * 6.0, mover.pos.y + mover.facing.cos() * 6.0, mover.pos.z);
                info!("sweep: target {other:?} ({})", near.map_or("a spot 6 m ahead".to_string(), |(d, _)| format!("{d:.1} m")));
                let free = sweep.plan(&book, id, me.obj_id, other, point, now + PAUSE);
                sweep.free_at = free;
            }
            None if real.elapsed_secs_f64() - sweep.last_shot > 2.0 => {
                info!("sweep: done, {} frames", sweep.taken);
                exit.write(AppExit::Success);
            }
            None => {}
        }
    }
}
