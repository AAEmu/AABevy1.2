//! What the buffs on a unit look like: the aura of the buff's effect group (the `_dot` effects of the skills that apply it), the pose
//! it holds the unit in (stun, struggle, levitation, an emote, a song) and the animation freeze of ice and stone.
//!
//! Known native-folded instances perform animation operations in receipt order.
//! Reconciliation retains their decisions and distance-limits FX; it does not
//! turn a denied start into a later visibility-driven retry. Pending host
//! controllers still do not establish native FIFO/elapsed-pose equivalence.
//! The data is `buffs` (see [`BuffLook`]):
//!
//! * `fx_group_id`: items that end at [`ev::END`] live as long as the buff (a held effect, `Fx` with a long life), the other ones
//!   play once when it arrives, and items that start at END play when it goes (`ice_piece_dot_end`, `pursuer_mark_dot_end`).
//! * `anim_start_id`: the clip of the pose (`all_re_combat_stun`, `all_re_combat_struggle`, `all_co_sk_hovering`, `fist_em_fear`):
//!   `anims.loop` clips loop for as long as the buff lasts, the others play once. A song buff plays the cast pose of the instrument
//!   held. `anim_end_id` plays when it ends (`all_re_stop` and the other stop markers only end the pose).
//! * `framehold`: the unit's animation freezes. `ragdoll`: the unit falls over (there is no ragdoll: the death clip plays and holds)
//!   and gets up early, so the stand-up ends with the buff (`ClientBuffMan::Update` calls the actor's stand-up as soon as the time left
//!   is no longer than the clip, `docs/research/re-skills.md` section 2).
//! * `knock_down` (163 buffs, no clip in the data): the unit is thrown: `all_re_combat_knockback_start` plays once (the root motion of the
//!   clip is the throw), then `all_re_combat_knockback` lies there; `_b` clips instead when the buff's caster stands behind the unit
//!   (`dot(owner - caster, forward) > 0`: it falls forward). When the buff ends the unit gets up with `standup_nw_back` (`standup_nw_stomach`
//!   for the `_b` fall) and only then moves again.
//!
//! Units farther than [`NEAR`] from the player show nothing (the effects go, the buffs stay in the list).

use super::*;

/// Units within this distance (m) of the player show their buffs; beyond `* 1.2` the effects go again.
const NEAR: f32 = 60.0;

/// The buffs of a unit that show something.
#[derive(Component, Default)]
pub(super) struct BuffVisuals {
    active: Vec<Active>,
    /// The unit is far: nothing is shown.
    far: bool,
}

struct Active {
    index: u32,
    id: u32,
    look: Arc<skills::BuffLook>,
    /// Effects that live as long as the buff.
    held: Vec<Entity>,
    /// The buff's clip is the unit's action (`Acting`).
    posed: bool,
    /// How the unit lies when the buff throws it (`knock_down`): on its back, or on its front.
    lying: Option<Lying>,
    /// The stand-up of a `ragdoll` buff has started (it ends with the buff).
    rising: bool,
}

#[cfg(test)]
pub(super) fn fixture_knockdown(look: Arc<skills::BuffLook>) -> BuffVisuals {
    BuffVisuals { far: false, active: vec![Active { index: 7, id: 1172, look,
        held: Vec::new(), posed: true, lying: Some(Lying::Back), rising: false }] }
}

#[cfg(test)]
pub(super) fn fixture_records(visual: &BuffVisuals) -> Vec<(u32, u32, bool, Vec<Entity>)> {
    visual.active.iter().map(|a| (a.index, a.id, a.posed, a.held.clone())).collect()
}

#[cfg(test)]
pub(super) fn fixture_held(visual: &mut BuffVisuals, entity: Entity) { visual.active[0].held.push(entity); }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lying {
    Back,
    Front,
}

impl Lying {
    /// `[start clip, the clip it lies in, the stand-up clip]`.
    fn clips(self) -> [&'static str; 3] {
        match self {
            Lying::Back => ["all_re_combat_knockback_start", "all_re_combat_knockback", "standup_nw_back"],
            Lying::Front => ["all_re_combat_knockback_b_start", "all_re_combat_knockback_b", "standup_nw_stomach"],
        }
    }
}

/// The seconds a pose that lasts as long as its buff may hold the unit (the buff ends it long before).
const POSE_HOLD: f64 = 1.0e7;

/// Populate before the receipt loop, so several operations in one batch share
/// one component instead of waiting for deferred first-sight insertion.
#[allow(clippy::type_complexity)]
pub(super) fn ensure_visuals(mut commands: Commands, actors: Query<Entity, (
    Without<BuffVisuals>, Or<(With<RemoteUnit>, With<LocalPlayer>)>,
    Or<(With<Animator>, With<super::super::units::Asleep>)>,
)>) {
    for entity in &actors { commands.entity(entity).insert(BuffVisuals::default()); }
}

fn near(st: &Stage, unit: Entity, was_far: bool) -> bool {
    let Some(here) = st.local.single().ok().and_then(|(e, _)| st.xforms.get(e).ok()).map(|g| g.translation()) else { return false };
    st.xforms.get(unit).is_ok_and(|g| g.translation().distance(here) <= if was_far { NEAR } else { NEAR * 1.2 })
}

fn retire_fx(st: &mut Stage, active: &mut Active) {
    for entity in active.held.drain(..) { st.commands.entity(entity).try_despawn(); }
}

fn update_distance(st: &mut Stage, unit: Entity, visual: &mut BuffVisuals) {
    let far = !near(st, unit, visual.far);
    if visual.far == far { return; }
    for active in &mut visual.active {
        retire_fx(st, active);
        if !far { active.held = spawn_fx(st, unit, &active.look, true); }
    }
    visual.far = far;
}

/// Actual Apply/Remove animation work shares the receipt loop with emotes and
/// skills. A rejected pose is remembered; visibility never retries that start.
pub(super) fn at_receipt(
    change: super::framehold::BuffChange,
    holds: &super::framehold::Frameholds,
    st: &mut Stage,
    visuals: &mut Query<&mut BuffVisuals>,
    now: f64,
) {
    match change {
        super::framehold::BuffChange::None => {},
        super::framehold::BuffChange::Seed { obj, entries } => {
            let Some(unit) = st.entity(obj) else { return };
            let Ok(mut visual) = visuals.get_mut(unit) else { return };
            for active in &mut visual.active { retire_fx(st, active); }
            visual.active.clear();
            visual.far = !near(st, unit, false);
            // Native initialization applies Good/Bad/Hidden in physical order.
            // This is a new actor, not a visibility-driven hold reassertion.
            if let Ok((mut animator, ..)) = st.actors.get_mut(unit) { animator.set_framehold(false); }
            for entry in entries {
                let look = st.book.buff(entry.buff.buff_id).unwrap_or_default();
                if look.framehold && let Ok((mut animator, ..)) = st.actors.get_mut(unit) { animator.set_framehold(true); }
                let active = add_buff(st, unit, entry.buff.index, entry.buff.buff_id,
                    entry.buff.caster.obj_id, look, now, !visual.far, holds.pose_allowed(obj, &entry.buff));
                visual.active.push(active);
            }
            if let Ok((mut animator, ..)) = st.actors.get_mut(unit) { animator.set_framehold(holds.held(obj)); }
        },
        super::framehold::BuffChange::Created { obj, previous, entry } => {
            let Some(unit) = st.entity(obj) else { return };
            let Ok(mut visual) = visuals.get_mut(unit) else { return };
            update_distance(st, unit, &mut visual);
            let look = st.book.buff(entry.buff.buff_id).unwrap_or_default();
            if let Some(active) = visual.active.iter_mut().find(|active|
                active.id == entry.buff.buff_id && active.index == previous.unwrap_or(entry.buff.index))
            {
                active.index = entry.buff.index;
                // A rekey retains its held FX; an admitted repeat really
                // attempts the start again. Rejection retains old ownership.
                if holds.pose_allowed(obj, &entry.buff) {
                    let (posed, lying) = start_pose(st, unit, entry.buff.caster.obj_id, &look, now);
                    active.posed |= posed;
                    if posed { active.lying = lying; active.rising = false; }
                }
            } else {
                let active = add_buff(st, unit, entry.buff.index, entry.buff.buff_id,
                    entry.buff.caster.obj_id, look, now, !visual.far, holds.pose_allowed(obj, &entry.buff));
                visual.active.push(active);
            }
        },
        super::framehold::BuffChange::Removed { obj, entry } => {
            let Some(unit) = st.entity(obj) else { return };
            let Ok(mut visual) = visuals.get_mut(unit) else { return };
            update_distance(st, unit, &mut visual);
            if let Some(i) = visual.active.iter().position(|active| active.index == entry.buff.index && active.id == entry.buff.buff_id) {
                let active = visual.active.remove(i);
                remove_buff(st, unit, active, !visual.far, now);
            }
        },
    }
}

/// Reconcile retained manager membership and distance-limited FX. Existing
/// Active records survive distance culling so denied/consumed poses cannot
/// become new starts when the camera returns.
#[allow(clippy::type_complexity)]
pub(super) fn reconcile_buffs(
    time: Res<Time>,
    holds: Res<super::framehold::Frameholds>,
    mut st: Stage,
    units: Query<(Entity, Option<&RemoteUnit>, Option<&LocalPlayer>), Or<(With<Animator>, With<super::super::units::Asleep>)>>,
    mut visuals: Query<&mut BuffVisuals>,
) {
    let now = time.elapsed_secs_f64();
    for (unit, remote, local) in &units {
        let Some(obj) = local.map(|p| p.obj_id).or(remote.map(|p| p.obj_id)) else { continue };
        let Ok(mut visual) = visuals.get_mut(unit) else { continue };
        update_distance(&mut st, unit, &mut visual);
        let show = !visual.far;
        let Some(wanted) = holds.entries(obj) else { continue };
        rise_early(&mut st, unit, &mut visual, wanted, now);
        let gone: Vec<_> = (0..visual.active.len()).rev().filter(|&i| !wanted.iter()
            .any(|w| w.buff.index == visual.active[i].index && w.buff.buff_id == visual.active[i].id)).collect();
        for i in gone { let active = visual.active.remove(i); remove_buff(&mut st, unit, active, show, now); }
        for entry in wanted {
            if visual.active.iter().any(|a| a.index == entry.buff.index && a.id == entry.buff.buff_id) { continue; }
            let look = st.book.buff(entry.buff.buff_id).unwrap_or_default();
            let active = add_buff(&mut st, unit, entry.buff.index, entry.buff.buff_id,
                entry.buff.caster.obj_id, look, now, show, holds.pose_allowed(obj, &entry.buff));
            visual.active.push(active);
        }
    }
}

fn spawn_fx(st: &mut Stage, unit: Entity, look: &skills::BuffLook, held_only: bool) -> Vec<Entity> {
    let mut held = Vec::new();
    for item in look.fx.iter().filter(|i| i.start < ev::END || i.start == ev::VOICE) {
        let hold = item.end == ev::END && item.start != ev::VOICE && (!item.effect.is_empty() || matches!(item.sound, Some(Sfx::Event(_))));
        if held_only && !hold { continue; }
        let spawned = if hold { st.spawn_held(item, unit) } else { st.spawn_item(item, unit, None, false) };
        if hold && let Some(e) = spawned { held.push(e); }
    }
    held
}

#[allow(clippy::too_many_arguments)]
fn add_buff(st: &mut Stage, unit: Entity, index: u32, id: u32, source: u32, look: Arc<skills::BuffLook>, now: f64, show_fx: bool, allow_pose: bool) -> Active {
    let held = if show_fx { spawn_fx(st, unit, &look, false) } else { Vec::new() };
    let (posed, lying) = if allow_pose { start_pose(st, unit, source, &look, now) } else { (false, None) };
    Active { index, id, look, held, posed, lying, rising: false }
}

fn start_pose(st: &mut Stage, unit: Entity, source: u32, look: &skills::BuffLook, now: f64) -> (bool, Option<Lying>) {
    if look.framehold { return (false, None); }
    if look.knock_down && !look.ragdoll && look.start_anim.is_none() {
        let lying = if thrown_forward(st, unit, source) { Lying::Front } else { Lying::Back };
        let posed = throw(st, unit, lying, now);
        (posed, posed.then_some(lying))
    } else { (pose(st, unit, look, now), None) }
}

/// The buff's caster stands behind the unit (`dot(owner - caster, forward) > 0`), so the unit falls on its face. A buff nobody cast
/// (or cast by the unit itself) has no direction: the unit falls on its back.
fn thrown_forward(st: &Stage, unit: Entity, source: u32) -> bool {
    let (Some(from), Some(owner)) = (st.entity(source).filter(|e| *e != unit).and_then(|e| st.xforms.get(e).ok()), st.xforms.get(unit).ok()) else { return false };
    let dir = (owner.translation() - from.translation()) * Vec3::new(1.0, 0.0, 1.0);
    dir.length_squared() > 1e-6 && dir.dot(*owner.forward()) > 0.0
}

/// The unit is thrown: the start clip plays once, then the unit lies in its last pose until the buff goes.
fn throw(st: &mut Stage, unit: Entity, lying: Lying, now: f64) -> bool {
    let [start, lie, _] = lying.clips();
    let Some(length) = st.play(unit, start, Act::Cast, false, POSE_HOLD, now) else { return false };
    if let (Some((first, _)), Some((cycle, _))) = (st.clip(unit, start), st.clip(unit, lie))
        && let Ok((mut animator, ..)) = st.actors.get_mut(unit)
    {
        animator.play_once_then(first, cycle, 0.08);
    }
    if verbose() {
        info!("combat: [{now:.3}] knockdown {lying:?}: {start} {length:.2} s");
    }
    st.commands.entity(unit).try_insert(Acting { until: now + POSE_HOLD, layered: false });
    true
}

/// Gets the unit up: the stand-up clip plays (to the end), then the unit moves again.
fn stand_up(st: &mut Stage, unit: Entity, clip: &str, now: f64) {
    // The old buff action ended even if the new stand-up start is rejected.
    // Native admission drops the new start; it does not queue it for release.
    let hold = st.clip(unit, clip).map_or(0.0, |(c, _)| f64::from(c.clip.duration) * 0.98);
    st.commands.entity(unit).try_remove::<Acting>();
    if hold > 0.0 && st.play(unit, clip, Act::Cast, false, hold, now).is_some() {
        st.commands.entity(unit).try_insert(Acting { until: now + hold, layered: false });
    } else {
        st.commands.queue(move |w: &mut World| {
            if let Some(mut l) = w.get_mut::<Locomotion>(unit) {
                l.invalidate();
            }
        });
    }
}

/// A `ragdoll` buff's stand-up starts when the time left is the clip's length, so it ends with the buff.
fn rise_early(st: &mut Stage, unit: Entity, visual: &mut BuffVisuals, entries: &[aa_net::state::BuffEntry], now: f64) {
    if !st.actors.get(unit).is_ok_and(|actor| actor.0.starts_enabled()) { return; }
    for active in visual.active.iter_mut().filter(|a| a.look.ragdoll && a.posed && !a.rising) {
        let Some(left) = entries.iter().find(|e| e.buff.index == active.index).and_then(|e| e.remaining_ms()) else { continue };
        let Some(length) = st.clip(unit, "standup_nw_back").map(|(c, _)| f64::from(c.clip.duration)) else { continue };
        if (left as f64) / 1000.0 <= length {
            active.rising = true;
            stand_up(st, unit, "standup_nw_back", now);
        }
    }
}

/// A buff went (`play_end`: the unit is near enough to see it): the effects end, the pose releases the unit.
fn remove_buff(st: &mut Stage, unit: Entity, active: Active, play_end: bool, now: f64) {
    for e in active.held {
        st.commands.entity(e).try_despawn();
    }
    if play_end {
        for item in active.look.fx.iter().filter(|i| i.start == ev::END && !i.effect.is_empty()) {
            st.spawn_item(item, unit, None, false);
        }
    }
    if let Some(lying) = active.lying {
        // the unit gets up (the buff's end clip is a stop marker)
        stand_up(st, unit, lying.clips()[2], now);
    } else if active.posed && active.rising {
        // the stand-up of a ragdoll buff is already playing
    } else if active.posed {
        st.commands.entity(unit).try_remove::<Acting>();
        st.commands.queue(move |w: &mut World| {
            if let Some(mut l) = w.get_mut::<Locomotion>(unit) {
                l.invalidate();
            }
        });
    }
    // Native end commands do not require successful ownership of the start.
    // Stop markers were already handled at the receipt; a knockdown's host
    // stand-up path above remains separate from ordinary end clips. FX
    // distance does not suppress an otherwise admitted animation command.
    if active.lying.is_none() && !active.rising
        && let Some(end) = active.look.end_anim.as_ref().filter(|end| !skills::is_stop_clip(end))
    { st.play(unit, end, Act::Cast, false, 0.0, now); }
}

/// The unit holds the pose of a buff. Returns whether it plays one.
fn pose(st: &mut Stage, unit: Entity, look: &skills::BuffLook, now: f64) -> bool {
    let wield = st.wield(unit);
    let clip = wield.instrument.and_then(|k| look.instrument_start.get(k)).and_then(|c| c.as_ref()).or(look.start_anim.as_ref()).cloned();
    // a unit that falls over lies in its death clip
    // (the player's `.cal` has `dead_02`, the creatures' `dead_01`)
    let clip = clip.or_else(|| look.ragdoll.then(|| ["dead_01", "dead_02"].into_iter().find(|n| st.clip(unit, n).is_some()).map(Arc::from)).flatten());
    let Some(clip) = clip else { return false };
    let looping = st.book.clip_loops(&clip);
    let played = st.play(unit, &clip, Act::Cast, looping, POSE_HOLD, now).is_some();
    // the reaction clips come as a start and a loop (`all_re_combat_stun_start` falls into `all_re_combat_stun`)
    if played
        && looping
        && let Some((start, _)) = st.clip(unit, &format!("{clip}_start"))
        && let Some((cycle, _)) = st.clip(unit, &clip)
        && let Ok((mut animator, ..)) = st.actors.get_mut(unit)
    {
        animator.play_once_then(start, cycle, 0.08);
    }
    if played && !looping {
        // a clip that does not loop holds its last frame for as long as the buff lasts (a fall, a death)
        st.commands.entity(unit).try_insert(Acting { until: now + POSE_HOLD, layered: false });
    }
    played
}
