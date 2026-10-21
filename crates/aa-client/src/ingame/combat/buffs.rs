//! What the buffs on a unit look like: the aura of the buff's effect group (the `_dot` effects of the skills that apply it), the pose
//! it holds the unit in (stun, struggle, levitation, an emote, a song) and the animation freeze of ice and stone.
//!
//! The server only says which buffs a unit has (`aa_net::state::Buffs`: the unit-state packet and `SCBuffCreated` /
//! `SCBuffRemoved`), so this reconciles what the unit shows with that list every frame; a buff that was there when the unit came
//! into view therefore shows exactly like one that was applied in front of the player. The data is `buffs` (see [`BuffLook`]):
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

use aa_net::state::Buffs;

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

/// Shows the buffs of the units around the player (system; runs before locomotion).
#[allow(clippy::type_complexity)]
pub(super) fn reconcile_buffs(
    time: Res<Time>,
    buffs: Option<Res<Buffs>>,
    mut st: Stage,
    units: Query<(Entity, Option<&RemoteUnit>, Option<&LocalPlayer>), With<Animator>>,
    mut visuals: Query<&mut BuffVisuals>,
) {
    let Some(buffs) = buffs else { return };
    let now = time.elapsed_secs_f64();
    let Some(here) = st.local.single().ok().and_then(|(e, _)| st.xforms.get(e).ok()).map(|g| g.translation()) else { return };
    for (unit, remote, local) in &units {
        let Some(obj) = remote.map(|r| r.obj_id).or(local.map(|l| l.obj_id)) else { continue };
        let Ok(mut visual) = visuals.get_mut(unit) else {
            // first sight of the unit: the next frame has the component
            if !buffs.of(obj).is_empty() {
                st.commands.entity(unit).try_insert(BuffVisuals::default());
            }
            continue;
        };
        rise_early(&mut st, unit, &mut visual, buffs.of(obj), now);
        let distance = st.xforms.get(unit).map_or(0.0, |g| g.translation().distance(here));
        let far = distance > if visual.far { NEAR } else { NEAR * 1.2 };
        let wanted: &[aa_net::state::BuffEntry] = if far { &[] } else { buffs.of(obj) };
        // nothing changed: the same buffs, in the same state
        let same = visual.far == far && visual.active.len() == wanted.len() && wanted.iter().all(|w| visual.active.iter().any(|a| a.index == w.buff.index && a.id == w.buff.buff_id));
        if same {
            continue;
        }
        visual.far = far;
        // what went
        let gone: Vec<usize> = (0..visual.active.len()).rev().filter(|&i| !wanted.iter().any(|w| w.buff.index == visual.active[i].index && w.buff.buff_id == visual.active[i].id)).collect();
        for i in gone {
            let active = visual.active.remove(i);
            remove_buff(&mut st, unit, active, !far, now);
        }
        // what came, in the server's order
        for w in wanted {
            if visual.active.iter().any(|a| a.index == w.buff.index && a.id == w.buff.buff_id) {
                continue;
            }
            // a buff that shows nothing is remembered all the same (the list is compared by id)
            let look = st.book.buff(w.buff.buff_id).unwrap_or_default();
            let active = add_buff(&mut st, unit, w.buff.index, w.buff.buff_id, w.buff.caster.obj_id, look, now);
            visual.active.push(active);
        }
        refreeze(&mut st, unit, &visual);
        if verbose() {
            info!("combat: [{now:.3}] buffs of {obj}: {:?}", visual.active.iter().map(|a| (a.id, a.held.len(), a.posed)).collect::<Vec<_>>());
        }
    }
}

/// A buff arrived: its effects start, its pose plays.
fn add_buff(st: &mut Stage, unit: Entity, index: u32, id: u32, source: u32, look: Arc<skills::BuffLook>, now: f64) -> Active {
    let mut held = Vec::new();
    for item in look.fx.iter().filter(|i| i.start < ev::END || i.start == ev::VOICE) {
        // an item that ends with the buff lives as long as it: an effect, and a sound (Dash's `skill_rush_speed` loops until the buff
        // goes, so its entity is kept and despawned with the buff)
        let hold = item.end == ev::END && item.start != ev::VOICE && (!item.effect.is_empty() || matches!(item.sound, Some(Sfx::Event(_))));
        let spawned = if hold { st.spawn_held(item, unit) } else { st.spawn_item(item, unit, None, false) };
        if hold && let Some(e) = spawned {
            held.push(e);
        }
    }
    let mut lying = None;
    let posed = if look.knock_down && !look.ragdoll && look.start_anim.is_none() {
        lying = Some(if thrown_forward(st, unit, source) { Lying::Front } else { Lying::Back });
        lying.is_some_and(|l| throw(st, unit, l, now))
    } else {
        pose(st, unit, &look, now)
    };
    Active { index, id, look, held, posed, lying, rising: false }
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
        // the clip the buff plays when it ends (getting up, the end of a bow)
        if play_end && let Some(end) = &active.look.end_anim {
            st.play(unit, end, Act::Cast, false, 0.0, now);
        }
    }
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

/// The animation of a unit with a `framehold` buff is frozen, any other one runs.
fn refreeze(st: &mut Stage, unit: Entity, visual: &BuffVisuals) {
    let frozen = visual.active.iter().any(|a| a.look.framehold);
    if let Ok((mut animator, ..)) = st.actors.get_mut(unit)
        && animator.paused != frozen
    {
        animator.paused = frozen;
    }
}
