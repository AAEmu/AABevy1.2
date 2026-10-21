//! Warms what a skill needs the first time it is used, so that nothing hitches when the player presses it: the clips of the player's
//! rig (decoded and bound) and the particle effects (their library parsed, textures and meshes loaded, materials created).
//!
//! It reads the action bar (`aa_net::state::ActionBar`) and, for every skill on it, collects the clips (cast, fire, two handed, dual
//! wield, controller, instrument, every plot step, their moving upper body variants), the effect groups (the skill's, its levels',
//! its projectile's, its plot steps', the buffs it applies) and queues them; a few are done per frame while frames are cheap.
//! Effects are started on the player invisible (`alpha_scale` 0) and die at once: that is all the first real use would cost.
//! `AA_COMBAT_PRELOAD=ID,ID` adds skills that are not on the bar (tests). `AA_COMBAT_NO_PRELOAD=1` turns it off.

use std::collections::{HashSet, VecDeque};
use web_time::Instant;

use aa_net::state::ActionBar;

use super::super::anim::Stance;
use super::*;

/// Milliseconds a frame spends binding clips at most.
const CLIP_BUDGET_MS: f64 = 2.0;
/// A frame that took longer than this (s) preloads nothing.
const BUSY_FRAME: f32 = 0.03;
/// Seconds a warm-up effect lives.
const FX_LIFE: f32 = 1.2;

#[derive(Resource, Default)]
pub(super) struct Preload {
    skills: HashSet<u32>,
    clips: VecDeque<String>,
    fx: VecDeque<(String, f32)>,
    seen_clips: HashSet<String>,
    seen_fx: HashSet<String>,
    /// The hit reactions, the death and the stances were queued.
    basics: bool,
    /// What was done, for the log.
    clips_done: usize,
    fx_done: usize,
}

/// `AA_COMBAT_NO_PRELOAD=1`.
pub(super) fn disabled() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("AA_COMBAT_NO_PRELOAD").is_some())
}

impl Preload {
    fn clip(&mut self, name: &str) {
        if !name.is_empty() && self.seen_clips.insert(name.to_string()) {
            self.clips.push_back(name.to_string());
        }
    }

    fn items(&mut self, items: &[FxItem]) {
        for item in items.iter().filter(|i| !i.effect.is_empty()) {
            if self.seen_fx.insert(item.effect.to_ascii_lowercase()) {
                self.fx.push_back((item.effect.clone(), item.scale));
            }
        }
    }

    /// Queues what skill `id` needs for a character with `wield`.
    fn skill(&mut self, book: &SkillBook, id: u32, wield: Wield) {
        let Some(v) = book.skill(id) else { return };
        let mut names: Vec<Arc<str>> = [&v.start_anim, &v.fire_anim, &v.twohand_fire_anim, &v.dual_fire_anim, &v.controller_anim].into_iter().flatten().cloned().collect();
        names.extend(v.instrument_start_clip(wield.instrument).cloned());
        names.extend(v.instrument_fire_clip(wield.instrument).cloned());
        self.items(&v.fx);
        if let Some(p) = &v.projectile {
            self.items(&p.items);
        }
        for (_, look) in &v.levels {
            self.items(&look.fx);
            if let Some(p) = &look.projectile {
                self.items(&p.items);
            }
        }
        if v.plot {
            for step in book.plot_steps(id) {
                let Some(p) = book.plot_event(step) else { continue };
                names.extend(p.clip().cloned());
                self.items(&p.fx);
                if let Some(p) = &p.projectile {
                    self.items(&p.items);
                }
            }
        }
        for buff in book.skill_buffs(id) {
            let Some(b) = book.buff(buff) else { continue };
            self.items(&b.fx);
            names.extend(b.start_anim.iter().cloned());
            names.extend(b.end_anim.iter().cloned());
            names.extend(wield.instrument.and_then(|k| b.instrument_start.get(k)).and_then(|c| c.clone()));
        }
        for name in names {
            self.clip(&name);
            // the clip a moving caster plays on its upper body
            if let Some(upper) = book.upper_clip(&name) {
                for candidate in skills::upper_candidates(&upper) {
                    self.clip(&candidate);
                }
            }
        }
    }
}

/// Queues the skills of the action bar and works the queue off (system).
#[allow(clippy::too_many_arguments)]
pub(super) fn run_preload(
    mut commands: Commands,
    real: Res<Time<Real>>,
    bar: Option<Res<ActionBar>>,
    book: Res<SkillBook>,
    library: Res<ClipLibrary>,
    player: Query<(Entity, &Animator, &Locomotion, Option<&Wield>), With<LocalPlayer>>,
    mut preload: ResMut<Preload>,
) {
    let Ok((me, animator, loco, wield)) = player.single() else { return };
    let wield = wield.copied().unwrap_or_default();
    // the skills of the bar, and the ones the test asked for
    let mut wanted: Vec<u32> = bar.iter().flat_map(|b| b.slots.iter().filter_map(|s| s.skill_id())).collect();
    if let Some(extra) = std::env::var("AA_COMBAT_PRELOAD").ok() {
        wanted.extend(extra.split(',').filter_map(|s| s.trim().parse::<u32>().ok()));
    }
    for id in wanted {
        if preload.skills.insert(id) {
            preload.skill(&book, id, wield);
        }
    }
    if !preload.basics {
        preload.basics = true;
        // what every fight needs: the hit reactions, the death, putting the weapon away
        for name in ["all_re_combat_hit", "all_re_combat_hit_l", "all_re_combat_hit_r", "all_re_combat_hit_mub", "all_re_combat_critical", "dead_01", "onehand_ac_holster_side_l", "twohand_ac_holster_back_r"] {
            preload.clip(name);
        }
        // the combat stances of the weapons the character carries (idle and the eight way run)
        let stances: Vec<Stance> = [wield.melee.then(|| if wield.staff { Stance::Staff } else if wield.two_handed { Stance::Twohand } else { Stance::Onehand }), Some(Stance::Fist), wield.instrument.map(|_| Stance::Music)].into_iter().flatten().collect();
        for stance in stances {
            let _ = loco.clips.stance(&animator.rig, &library, stance);
        }
    }
    if real.delta_secs() > BUSY_FRAME {
        return;
    }
    // clips: decoded and bound to the rig of the player (the decoded data is shared by every rig)
    let started = Instant::now();
    while started.elapsed().as_secs_f64() * 1000.0 < CLIP_BUDGET_MS {
        let Some(name) = preload.clips.pop_front() else { break };
        if bind_action_clip(&animator.rig, &loco.clips.cal, &library, &name).is_some() {
            preload.clips_done += 1;
        }
    }
    // effects: one per frame
    if let Some((name, scale)) = preload.fx.pop_front() {
        preload.fx_done += 1;
        let effect = Effect { scale, alpha_scale: 0.0, prime: false, looping: false, despawn_when_finished: true, ..Effect::new(name.clone()) };
        commands.spawn((Name::new(format!("preload {name}")), effect, Transform::IDENTITY, ChildOf(me), Fx { follow: None, fixed_rotation: false, ttl: FX_LIFE }));
    }
    if verbose() && preload.clips.is_empty() && preload.fx.is_empty() && (preload.clips_done > 0 || preload.fx_done > 0) {
        info!("combat: preload done: {} skills, {} clips, {} effects", preload.skills.len(), preload.clips_done, preload.fx_done);
        preload.clips_done = 0;
        preload.fx_done = 0;
    }
}
