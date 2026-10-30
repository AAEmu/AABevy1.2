//! Skills that move units: dashes and charges, leaps and back steps, pulls, knockbacks, blinks.
//!
//! In 1.2 the server does not move player characters for a skill ("the 1.2 client moves itself": the server's skill controllers only
//! run on non-character units and its `KnockBack` leaves characters alone), and it tells a client what to do with the data of the
//! skill alone:
//!
//! | Data | Server packet | What the client does |
//! |---|---|---|
//! | plot step with a `SkillController` effect (Charge, Overwhelm, Lasso, Vicious Implosion, Tiger Strike) | `SCPlotEvent` (sent for every step with a special or controller effect): step source and target | the unit the effect's `source_id` names runs the controller towards the unit `target_id` names |
//! | `skills.skill_controller_id` of a skill without a plot (Drop Back) | `SCSkillFired` | the caster runs it |
//! | `buffs.skill_controller_id` (Mana Force's four buffs push their target, Float's buff lifts its owner) | `SCBuffCreated` | the buff's owner runs it, with no target (along its facing); a floating controller (kind 1) lifts the owner `value1` mm at `value2` mm/s and holds it until `SCBuffRemoved`, then it falls |
//! | `Blink` / `TeleportToUnit` special effect (Teleportation, Shadow Step) | `SCBlinkUnit` (obj id, distance, degree, the end point the server computed against the terrain) to the caster only | the caster is at that point |
//! | `KnockBack` special effect (Whirlwind Slash, Focal Concussion, Charged Bolt, Meteor Strike, ...) | `SCUnitDamaged` of the victims; the server itself moves an NPC victim with one stopping `SCOneUnitMovement` at the end | a character victim is pushed by its own client |
//!
//! This module follows the packets: [`observe`] (called by `combat::apply`, so the move starts when the clip does, also after the
//! weapon draw delay) queues the events that matter, `start_moves` turns them into a [`SkillMove`] on the unit, and
//! `drive_local` / `drive_remote` play the paths of [`path`] with the collision of `aa_world`:
//!
//! * the **local player** is moved by `drive_local` instead of `player::control_player` (input is locked while a move runs), sends
//!   its movement packets on the way (jump / fall state, velocity) and a standing one at the end, so the server and the other
//!   players see the same flight; a `Teleported` event from the server cancels it ([`player::follow_server`] puts the player there);
//! * a **remote NPC** that is knocked back has one report at the end: the move waits for that report (or, after
//!   [`REPORT_WAIT`], the predicted end point) and plays the flight to it, then holds the unit there. Remote controllers (other
//!   players, NPC leaps) arrive as movement reports and are played by [`remote::Track`]; they are only simulated with
//!   `AA_COMBAT_FAKE` (nothing else moves the unit) or `AA_SKILL_MOTION_SIM=1`;
//! * the **camera** pivot trails the player ([`CameraLag`], read by `camera::follow_player`) and catches up in a fraction of a
//!   second, so a charge or a blink is followed, not jumped.
//!
//! A controller id with no row in `skill_controllers` is no controller (the 1.2 client has no default: Bull Rush's 4433, Precision Strike's 703,
//! Triple Slash's 604 move nobody; `docs/research/re-skills.md` section 1), and a stunned, sleeping or rooted unit only starts Leap
//! controllers.
//!
//! `AA_SKILL_MOTION_LOG=1` logs every move.

mod book;
mod path;

use std::collections::{HashMap, HashSet};

use aa_anim::{Animator, ClipLibrary};
use aa_net::skill::{PlotEvent, SkillTarget, target_kind};
use aa_net::state::{Buffs, Units};
use aa_net::{Command, Event, MoveCommand, MoveState, NetCommand, Pos};
use aa_world::{TerrainQuery, WorldCollision};
use bevy::math::{Vec2, Vec3};
use bevy::prelude::*;

use super::anim::{Locomotion, Motion, bind_action_clip};
use super::combat::{Acting, Dead};
use super::tools::SkillTool;
use super::player::{LocalPlayer, Mover, floor_at, placement};
use super::units::RemoteUnit;
use book::{Blink, Book, Effect, Knock};
use path::{Arc as Flight, Controller};

/// The capsule that collides with walls on the way (the player's, `player.rs`) and the step it climbs.
const RADIUS: f32 = 0.35;
const HEIGHT: f32 = 1.8;
const STEP_UP: f32 = 0.5;
/// Seconds between the movement packets of a move.
const SEND_INTERVAL: f32 = 0.1;
/// Seconds a knocked unit that was hit stays down (the get-up clip) before it may move again.
const GET_UP: f32 = 0.45;
/// How long a knocked NPC waits for the server's report of where it landed before the predicted end point is used (s).
const REPORT_WAIT: f64 = 0.5;
/// Seconds a plot step's knockback also applies to units the step's skill hurts afterwards.
const KNOCK_WINDOW: f64 = 1.5;
/// The camera pivot catches up with the player at this rate (1/s), and may trail by at most this much (m).
const LAG_RATE: f32 = 7.0;
const LAG_MAX: f32 = 7.0;
/// How far behind its target Shadow Step puts the caster (m): what the lane server answers (`SCBlinkUnit`) for a straw doll.
const BEHIND: f32 = 1.3;

fn log() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("AA_SKILL_MOTION_LOG").is_some())
}

/// Nothing else moves a remote unit (the events are made up): simulate controllers on other units as well.
fn simulate_remote() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("AA_COMBAT_FAKE").is_some() || std::env::var_os("AA_SKILL_MOTION_SIM").is_some())
}

/// Server events `combat::apply` hands over (when it applies them, so the move and the clip start together).
#[derive(Resource, Default)]
pub struct Inbox(Vec<(f64, Event)>);

/// Queues `event` if it can move a unit.
pub fn observe(inbox: &mut Inbox, event: &Event, now: f64) {
    let relevant = match event {
        Event::SkillStarted(s) => !s.failed(),
        Event::SkillFired(_) | Event::BuffCreated { .. } | Event::BuffRemoved { .. } | Event::PlotEvent(_) | Event::UnitDamaged(_) | Event::BlinkUnit(_) | Event::Teleported { .. } | Event::Resurrected { .. } | Event::UnitDeath { .. } => true,
        _ => false,
    };
    if relevant {
        inbox.0.push((now, event.clone()));
    }
}

/// How far behind the player the camera pivot is (game coordinates): follows the player's moves by `-delta` and decays.
#[derive(Resource, Default)]
pub struct CameraLag(pub Vec3);

impl CameraLag {
    fn push(&mut self, delta: Vec3) {
        self.0 -= delta;
        let len = self.0.length();
        if len > LAG_MAX {
            self.0 *= LAG_MAX / len;
        }
    }

    fn decay(&mut self, dt: f32) {
        self.0 *= (-LAG_RATE * dt).exp();
        if self.0.length_squared() < 1e-6 {
            self.0 = Vec3::ZERO;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// The caster runs or jumps (Charge, Drop Back).
    Dash,
    /// The unit is pulled to someone (Lasso, Vicious Implosion).
    Pull,
    /// The unit is thrown off its feet.
    Knock,
    /// The unit is somewhere else at once (Teleportation, Shadow Step).
    Blink,
    /// The unit floats up and hangs there while its buff lasts (Float), then falls.
    Float,
}

/// A skill is moving this unit.
#[derive(Component)]
pub struct SkillMove {
    flight: Flight,
    kind: Kind,
    /// Seconds into the flight.
    t: f32,
    /// The unit turns to where it goes.
    rotate: bool,
    /// Seconds the unit stays put after landing (the get-up of a knockdown).
    recover: f32,
    landed: bool,
    sent: f32,
    /// 0 no clip yet, 1 the flight clip plays, 2 the get-up clip plays.
    clip_phase: u8,
    /// A floating controller (kind 1): where the unit hangs.
    hover: Option<Hover>,
    /// Only actual admitted kind1/2/4 controller producers set this. Blink and
    /// impulse approximations do not establish native actor+58 activity.
    controller_kind: Option<u32>,
}

/// `Floating` skill controller of a buff (`value1` mm up, at `value2` mm/s): the unit rises, hangs while the buff lasts and falls
/// when it goes. Measured on the real client with Float (buff 181, controller 5836 = 6000 / 18000): +6 m at about 19 m/s, then a fall
/// that takes a 6 m drop in 0.65 s (gravity 25).
#[derive(Clone, Copy, Debug)]
struct Hover {
    /// The buff instance that holds the unit up.
    index: u32,
    top: f32,
    speed: f32,
    released: bool,
    /// Fall speed (m/s).
    fall: f32,
}

impl SkillMove {
    fn new(flight: Flight, kind: Kind, rotate: bool) -> SkillMove {
        let recover = if kind == Kind::Knock { GET_UP } else { 0.0 };
        SkillMove { flight, kind, t: 0.0, rotate, recover, landed: false, sent: SEND_INTERVAL, clip_phase: 0, hover: None, controller_kind: None }
    }

    /// A settled skill move for the deck tests: only its presence matters there.
    #[cfg(test)]
    pub(crate) fn fixture() -> SkillMove {
        let at = Vec3::ZERO;
        let flight = Flight {
            from: at,
            to: at,
            dir: Vec2::ZERO,
            dist: 0.0,
            hs: 0.0,
            vs: 0.0,
            az: 0.0,
            eta: 1.0,
        };
        let mut mv = SkillMove::new(flight, Kind::Blink, false);
        mv.landed = true;
        mv
    }

    /// A unit that floats up from `at`.
    fn float(at: Vec3, index: u32, c: &Controller) -> SkillMove {
        let flight = Flight { from: at, to: at, dir: Vec2::ZERO, dist: 0.0, hs: 0.0, vs: 0.0, az: 0.0, eta: 1.0 };
        let mut mv = SkillMove::new(flight, Kind::Float, false);
        mv.hover = Some(Hover { index, top: at.z + c.v[0] as f32 * 0.001, speed: (c.v[1] as f32 * 0.001).max(1.0), released: false, fall: 0.0 });
        mv.controller_kind = Some(c.kind);
        mv
    }

    /// Native actor+58 activity is independent of movement keys or lock0. Float's
    /// matched removal ends its controller before the application's falling tail.
    pub(super) fn controller_activity(&self) -> bool {
        matches!(self.controller_kind, Some(2 | 4))
            || (self.controller_kind == Some(1) && self.hover.is_some_and(|h| !h.released))
    }

    /// Seconds until the move is over.
    fn remaining(&self) -> f32 {
        (self.flight.eta - self.t).max(0.0) + self.recover
    }
}

/// One step of a move.
struct Stepped {
    pos: Vec3,
    velocity: Vec3,
    /// The move is over (landed and recovered).
    done: bool,
    /// In the air.
    airborne: bool,
}

/// Moves `mv` on by `dt` from `pos`: along its path, stopped by walls, never under the floor.
fn advance(mv: &mut SkillMove, pos: Vec3, dt: f32, terrain: &TerrainQuery, collision: &WorldCollision) -> Stepped {
    if let Some(h) = &mut mv.hover {
        if !h.released {
            let z = (pos.z + h.speed * dt).min(h.top);
            let vz = if dt > 0.0 { (z - pos.z) / dt } else { 0.0 };
            return Stepped { pos: Vec3::new(pos.x, pos.y, z), velocity: Vec3::new(0.0, 0.0, vz), done: false, airborne: true };
        }
        // the buff is gone: the unit falls to the floor
        h.fall += path::GRAVITY * dt;
        let floor = floor_at(terrain, collision, pos, STEP_UP);
        let z = pos.z - h.fall * dt;
        let landed = floor.is_finite() && z <= floor;
        let z = if landed { floor } else { z };
        mv.landed = landed;
        return Stepped { pos: Vec3::new(pos.x, pos.y, z), velocity: Vec3::new(0.0, 0.0, -h.fall), done: landed, airborne: !landed };
    }
    if mv.landed {
        mv.recover -= dt;
        return Stepped { pos, velocity: Vec3::ZERO, done: mv.recover <= 0.0, airborne: false };
    }
    mv.t += dt;
    let want = mv.flight.at(mv.t);
    let mut xy = Vec2::new(want.x, want.y);
    let mut blocked = false;
    if mv.flight.dist > 0.0 {
        let pushed = collision.slide(pos, xy - Vec2::new(pos.x, pos.y), RADIUS, HEIGHT, STEP_UP);
        let reached = Vec2::new(pushed.pos.x, pushed.pos.y);
        // a wall in the way ends the move where it is
        blocked = pushed.blocked && reached.distance(xy) > 0.2;
        xy = reached;
    }
    let floor = floor_at(terrain, collision, Vec3::new(xy.x, xy.y, pos.z.max(want.z)), STEP_UP);
    let mut z = want.z;
    if floor.is_finite() {
        z = z.max(floor);
    }
    mv.landed = mv.t >= mv.flight.eta || blocked;
    if mv.landed && floor.is_finite() {
        z = floor;
    }
    let next = Vec3::new(xy.x, xy.y, z);
    let velocity = if dt > 0.0 { (next - pos) / dt } else { Vec3::ZERO };
    let airborne = !mv.landed && floor.is_finite() && z - floor > 0.15;
    Stepped { pos: next, velocity, done: mv.landed && mv.recover <= 0.0, airborne }
}

/// What the cast of a timeline was aimed at (the plot steps name only their own source and target).
#[derive(Clone, Copy, Default)]
struct CastInfo {
    caster: u32,
    target: u32,
    target_pos: Option<Vec3>,
}

/// A knocked NPC waits for the server's report of where it landed.
struct PendingKnock {
    obj: u32,
    from: Vec3,
    dir: Vec2,
    knock: Knock,
    deadline: f64,
}

#[derive(Resource, Default)]
struct Tracker {
    casts: HashMap<u16, CastInfo>,
    /// Knockbacks of a plot step (timeline, pusher, when) that also apply to the victims of its blows.
    plot_knocks: HashMap<u16, (u32, Vec<Knock>, f64)>,
    /// Victims already pushed by a timeline.
    pushed: HashSet<(u16, u32)>,
    pending: Vec<PendingKnock>,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Inbox>();
    let Some(book) = Book::open() else {
        warn!("skill motion: the game database is missing: skills move nobody");
        return;
    };
    app.insert_resource(book)
        .init_resource::<Tracker>()
        .init_resource::<CameraLag>()
        .add_systems(
            Update,
            (
                start_moves.after(super::seating::SeatSet::Follow).before(super::player::control_player),
                drive_local.after(super::player::control_player).after(super::seating::SeatSet::Pose)
                    .before(super::anim::drive_locomotion).before(super::camera::follow_player),
                drive_remote.after(super::units::move_units),
                dev_script.before(super::player::control_player),
                decay_lag.after(drive_local).before(super::camera::follow_player),
            )
                .run_if(super::world_screens),
        );
}

/// A unit a skill can move.
#[derive(Clone, Copy)]
struct Who {
    entity: Entity,
    pos: Vec3,
    yaw: f32,
    local: bool,
    /// An NPC or creature (the server moves it), not a player character.
    npc: bool,
}

#[derive(bevy::ecs::system::SystemParam)]
struct Roster<'w, 's> {
    locals: Query<'w, 's, (Entity, &'static LocalPlayer, &'static Mover)>,
    remotes: Query<'w, 's, (Entity, &'static RemoteUnit, Has<Dead>)>,
    units: Res<'w, Units>,
    buffs: Option<Res<'w, Buffs>>,
}

impl Roster<'_, '_> {
    fn who(&self, obj: u32) -> Option<Who> {
        if obj == 0 {
            return None;
        }
        if let Some((entity, _, m)) = self.locals.iter().find(|(_, l, _)| l.obj_id == obj) {
            return Some(Who { entity, pos: m.pos, yaw: m.facing, local: true, npc: false });
        }
        let (entity, r, dead) = self.remotes.iter().find(|(_, r, _)| r.obj_id == obj)?;
        (!dead).then(|| Who { entity, pos: r.pos, yaw: r.facing, local: false, npc: !r.is_character() })
    }

    /// A buff of the unit stuns, puts to sleep or roots it.
    fn rooted(&self, obj: u32, book: &Book) -> bool {
        self.buffs.as_ref().is_some_and(|b| b.of(obj).iter().any(|e| book.is_rooting(e.buff.buff_id)))
    }

    /// Where a unit is, drawn or not.
    fn point(&self, obj: u32) -> Option<Vec3> {
        self.who(obj).map(|w| w.pos).or_else(|| self.units.get(obj).map(|u| Vec3::new(u.pos.x, u.pos.y, u.pos.z)))
    }
}

fn pos3(p: Pos) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

fn skill_target(t: &SkillTarget) -> (u32, Option<Vec3>) {
    let obj = if t.kind == target_kind::UNIT || t.kind == target_kind::DOODAD { t.obj_id } else { 0 };
    (obj, t.pos.map(pos3))
}

/// Turns the queued server events into moves.
#[allow(clippy::too_many_arguments)]
fn start_moves(
    time: Res<Time>,
    mut inbox: ResMut<Inbox>,
    mut tracker: ResMut<Tracker>,
    book: Res<Book>,
    roster: Roster,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    let events = std::mem::take(&mut inbox.0);
    let mut start = Starter { book: &*book, roster: &roster, commands: &mut commands, tracker: &mut tracker, now };
    for (_, event) in events {
        match event {
            Event::SkillStarted(s) => {
                let (target, target_pos) = skill_target(&s.target);
                start.tracker.casts.insert(s.tl, CastInfo { caster: s.caster.obj_id, target, target_pos });
            }
            Event::SkillFired(f) => {
                let (target, target_pos) = skill_target(&f.target);
                start.tracker.casts.insert(f.tl, CastInfo { caster: f.caster.obj_id, target, target_pos });
                let moves = start.book.skill(f.skill_id);
                // a skill without a plot that has a controller (Drop Back)
                if let Some(c) = moves.controller {
                    start.controller(&c, f.caster.obj_id, (target != f.caster.obj_id && target != 0).then_some(target), target_pos, true);
                }
                // made-up events have no `SCBlinkUnit`: the blink the server would answer with
                if let (true, Some(blink)) = (simulate_remote(), moves.blink) {
                    start.sim_blink(blink, f.caster.obj_id, target, target_pos);
                }
            }
            // a buff with a controller starts it on its owner, with no target (Mana Force pushes along the owner's facing)
            Event::BuffCreated { obj_id, buff } => {
                if let Some(c) = start.book.buff_controller(buff.buff_id) {
                    if c.kind == 1 {
                        start.float(&c, obj_id, buff.index);
                    } else {
                        start.controller(&c, obj_id, None, None, true);
                    }
                }
            }
            // the buff that holds a unit up is gone: it falls
            Event::BuffRemoved { obj_id, index } => {
                if let Some(w) = start.roster.who(obj_id) {
                    start.commands.queue(move |world: &mut World| {
                        if let Some(mut mv) = world.get_mut::<SkillMove>(w.entity)
                            && let Some(h) = mv.hover.as_mut().filter(|h| h.index == index)
                        {
                            h.released = true;
                        }
                    });
                }
            }
            Event::PlotEvent(p) => start.plot(&p),
            Event::UnitDamaged(d) => start.damaged(&d),
            Event::BlinkUnit(b) => {
                let to = pos3(b.position());
                match start.roster.who(b.obj_id) {
                    // the server computed the end point against the terrain: the player is there at once, the camera follows
                    Some(w) if w.local => start.blink(w.entity, w.pos, to),
                    Some(w) => {
                        // no gliding across the gap: the unit is simply there
                        let entity = w.entity;
                        start.commands.queue(move |world: &mut World| {
                            if let Some(mut r) = world.get_mut::<RemoteUnit>(entity) {
                                let yaw = r.facing;
                                r.reset_track(to, yaw);
                            }
                        });
                    }
                    None => {}
                }
            }
            Event::Teleported { .. } => {
                for (entity, ..) in roster.locals.iter() {
                    start.commands.entity(entity).try_remove::<SkillMove>();
                }
            }
            Event::Resurrected { obj_id, .. } | Event::UnitDeath { obj_id, .. } => {
                if let Some(w) = start.roster.who(obj_id) {
                    start.commands.entity(w.entity).try_remove::<SkillMove>();
                }
                start.tracker.pending.retain(|p| p.obj != obj_id);
            }
            _ => {}
        }
    }
    // knocked NPCs: the server's report of the landing, else the prediction
    let pending = std::mem::take(&mut tracker.pending);
    for p in pending {
        let Some(who) = roster.who(p.obj) else { continue };
        let Ok((_, unit, _)) = roster.remotes.get(who.entity) else { continue };
        let latest = unit.latest();
        let moved = Vec2::new(latest.x - p.from.x, latest.y - p.from.y);
        if moved.length() >= p.knock.distance * 0.5 {
            // where the server put it
            let flight = path::knock(who.pos, moved.normalize_or_zero(), moved.length(), p.knock.angle);
            if log() {
                info!("skill motion: unit {} knocked {:.2} m (reported)", p.obj, moved.length());
            }
            commands.entity(who.entity).insert(SkillMove::new(flight, Kind::Knock, false));
        } else if now >= p.deadline {
            let flight = path::knock(who.pos, p.dir, p.knock.distance, p.knock.angle);
            if log() {
                info!("skill motion: unit {} knocked {:.2} m (predicted)", p.obj, p.knock.distance);
            }
            commands.entity(who.entity).insert(SkillMove::new(flight, Kind::Knock, false));
        } else {
            tracker.pending.push(p);
        }
    }
}

struct Starter<'a, 'w, 's, 'cw, 'cs> {
    book: &'a Book,
    roster: &'a Roster<'w, 's>,
    commands: &'a mut Commands<'cw, 'cs>,
    tracker: &'a mut Tracker,
    now: f64,
}

impl Starter<'_, '_, '_, '_, '_> {
    /// The player is at `to` at once.
    fn blink(&mut self, entity: Entity, from: Vec3, to: Vec3) {
        let flight = Flight { from, to, dir: Vec2::ZERO, dist: 0.0, hs: 0.0, vs: 0.0, az: 0.0, eta: 0.0 };
        self.commands.entity(entity).insert(SkillMove::new(flight, Kind::Blink, false));
        if log() {
            info!("skill motion: blink of the player to ({:.1}, {:.1}, {:.1})", to.x, to.y, to.z);
        }
    }

    /// The `SCBlinkUnit` a server would send for the blink of a skill the (made-up) events cast: the local player goes there through a
    /// very short dash, so walls and the floor stop it as the server's terrain query does.
    fn sim_blink(&mut self, blink: Blink, caster: u32, target: u32, target_pos: Option<Vec3>) {
        let Some(who) = self.roster.who(caster).filter(|w| w.local) else { return };
        let to = match blink {
            Blink::Forward(metres) => {
                let f = path::forward(who.yaw);
                who.pos + Vec3::new(f.x * metres, f.y * metres, 0.0)
            }
            Blink::Behind => {
                let Some(t) = (target != caster).then(|| self.roster.point(target).or(target_pos)).flatten() else { return };
                let away = Vec2::new(t.x - who.pos.x, t.y - who.pos.y).normalize_or_zero();
                t + Vec3::new(away.x * BEHIND, away.y * BEHIND, 0.0)
            }
        };
        let d = Vec2::new(to.x - who.pos.x, to.y - who.pos.y);
        let dist = d.length();
        const SECS: f32 = 0.05;
        let flight = Flight { from: who.pos, to, dir: d.normalize_or_zero(), dist, hs: dist / SECS, vs: 0.0, az: 0.0, eta: SECS };
        self.commands.entity(who.entity).insert(SkillMove::new(flight, Kind::Blink, false));
        if log() {
            info!("skill motion: blink of the player {dist:.1} m (made-up events)");
        }
    }

    /// A plot step: the controllers and knockbacks the database gives it.
    fn plot(&mut self, p: &PlotEvent) {
        let cast = *self.tracker.casts.entry(p.tl).or_insert(CastInfo { caster: p.caster_obj_id, target: p.target_obj_id, target_pos: p.target_pos.map(pos3) });
        if self.tracker.casts.len() > 256 {
            self.tracker.casts.clear();
            self.tracker.casts.insert(p.tl, cast);
        }
        let steps = self.book.step(p.event_id);
        for m in steps.iter() {
            let obj = |id: u8| match id {
                1 => cast.caster,
                2 => cast.target,
                3 => p.caster_obj_id,
                4 | 5 => p.target_obj_id,
                _ => 0,
            };
            let pos = |id: u8| match id {
                2 => cast.target_pos,
                3 => p.caster_pos.map(pos3),
                4 | 5 => p.target_pos.map(pos3),
                _ => None,
            };
            let (mover, reference) = (obj(m.source), obj(m.target));
            match m.effect {
                Effect::Controller(c) => {
                    let skill_caster = mover == cast.caster;
                    self.controller(&c, mover, (reference != mover && reference != 0).then_some(reference), pos(m.target), skill_caster);
                }
                Effect::Knock(k) => {
                    // the step's source pushes its target; the blows of the skill that follow push their victims too
                    self.knock(reference, mover, pos(m.source), k);
                    self.tracker.pushed.insert((p.tl, reference));
                    let entry = self.tracker.plot_knocks.entry(p.tl).or_insert((mover, Vec::new(), self.now));
                    entry.1.push(k);
                    entry.2 = self.now;
                }
            }
        }
        if self.tracker.pushed.len() > 512 {
            self.tracker.pushed.clear();
        }
    }

    /// The blows of a skill: its `KnockBack` effects push the victim.
    fn damaged(&mut self, d: &aa_net::skill::UnitDamage) {
        if d.action.kind > 1 || d.damage <= 0 || aa_net::skill::hit_type::is_miss(d.hit_type) {
            return;
        }
        let knocks: Vec<Knock> = if d.action.kind == 0 {
            self.book.skill(d.action.skill_id).knocks.clone()
        } else {
            match self.tracker.plot_knocks.get(&d.action.tl) {
                Some((_, knocks, at)) if self.now - at <= KNOCK_WINDOW => knocks.clone(),
                _ => return,
            }
        };
        if knocks.is_empty() || !self.tracker.pushed.insert((d.action.tl, d.target_obj_id)) {
            return;
        }
        for k in knocks {
            self.knock(d.target_obj_id, d.caster_obj_id, None, k);
        }
    }

    /// A floating controller of buff `index` lifts `mover`.
    fn float(&mut self, c: &Controller, mover: u32, index: u32) {
        let Some(who) = self.roster.who(mover) else { return };
        // other units move by the server's reports; only made-up events need the simulation
        if (!who.local && !simulate_remote()) || (self.roster.rooted(mover, self.book) && c.kind != 2) {
            return;
        }
        if log() {
            info!("skill motion: controller {} lifts unit {mover} {:.1} m at {:.1} m/s", c.id, c.v[0] as f32 * 0.001, c.v[1] as f32 * 0.001);
        }
        self.commands.entity(who.entity).insert(SkillMove::float(who.pos, index, c));
    }

    /// `mover` runs controller `c` towards `reference` (a unit, else the spot `at`, else itself).
    fn controller(&mut self, c: &Controller, mover: u32, reference_unit: Option<u32>, at: Option<Vec3>, caster: bool) {
        let Some(who) = self.roster.who(mover) else { return };
        // other units move by the server's reports; only made-up events need the simulation
        if !who.local && !simulate_remote() {
            return;
        }
        // a stunned, sleeping or rooted unit only starts Leap controllers (`StartSkillController` is skipped for the others)
        if c.kind != 2 && self.roster.rooted(mover, self.book) {
            return;
        }
        let target = reference_unit.and_then(|u| self.roster.point(u)).or(at);
        let flight = match c.kind {
            2 => path::leap_end(c, who.pos, who.yaw, target).and_then(|end| path::leap(c, who.pos, end, 1.0)),
            4 => path::dash(c, who.pos, who.yaw),
            _ => None,
        };
        let Some(flight) = flight else {
            if log() {
                info!("skill motion: controller {} keeps unit {mover} where it is", c.id);
            }
            return;
        };
        if log() {
            info!("skill motion: controller {} moves unit {mover} {:.1} m in {:.2} s ({:.1} m/s, lift {:.2} m)", c.id, flight.dist, flight.eta, flight.hs, flight.lift());
        }
        let kind = if caster { Kind::Dash } else { Kind::Pull };
        let mut movement = SkillMove::new(flight, kind, c.rotates());
        movement.controller_kind = Some(c.kind);
        self.commands.entity(who.entity).insert(movement);
    }

    /// `pusher` knocks `victim` back (from the spot `from` when the pusher is not a unit).
    fn knock(&mut self, victim: u32, pusher: u32, from: Option<Vec3>, k: Knock) {
        let Some(who) = self.roster.who(victim) else { return };
        let origin = self.roster.point(pusher).or(from);
        let away = origin.map(|o| Vec2::new(who.pos.x - o.x, who.pos.y - o.y)).filter(|d| d.length_squared() > 0.01).unwrap_or_else(|| path::forward(who.yaw));
        let dir = away.normalize_or_zero();
        if who.local {
            let flight = path::knock(who.pos, dir, k.distance, k.angle);
            if log() {
                info!("skill motion: the player is knocked {:.1} m", k.distance);
            }
            self.commands.entity(who.entity).insert(SkillMove::new(flight, Kind::Knock, false));
        } else if who.npc || simulate_remote() {
            // the server moves an NPC with one report at the end of the push
            self.tracker.pending.retain(|p| p.obj != victim);
            self.tracker.pending.push(PendingKnock { obj: victim, from: who.pos, dir, knock: k, deadline: self.now + REPORT_WAIT });
        }
    }
}

/// The clip a unit plays while a move runs, and at its end.
fn clip_for(kind: Kind, landed: bool) -> Option<(&'static str, bool)> {
    match (kind, landed) {
        (Kind::Knock, false) => Some(("all_re_combat_knockback_start", false)),
        (Kind::Knock, true) => Some(("all_re_combat_crash_end", false)),
        (Kind::Pull, false) => Some(("all_re_combat_knockback_b_start", false)),
        _ => None,
    }
}

/// Plays `name` on a unit as an action for `hold` seconds.
fn play(commands: &mut Commands, entity: Entity, animator: &mut Animator, loco: &Locomotion, clips: &ClipLibrary, name: &str, looping: bool, hold: f64, now: f64) -> Option<f32> {
    if !animator.starts_enabled() { return None; }
    let clip = bind_action_clip(&animator.rig, &loco.clips.cal, clips, name)?;
    let duration = clip.clip.duration;
    animator.stop_layer(0.0);
    animator.play(clip, looping, 0.06);
    animator.speed = 1.0;
    animator.random_idle = None;
    commands.entity(entity).try_remove::<SkillTool>().insert(Acting { until: now + hold, layered: false });
    Some(duration)
}

/// Starts the clip a move calls for (once at the start, once on landing).
fn start_clip(mv: &mut SkillMove, commands: &mut Commands, entity: Entity, animator: Option<Mut<Animator>>, loco: Option<&Locomotion>, clips: &ClipLibrary, now: f64) {
    let (Some(mut animator), Some(loco)) = (animator, loco) else { return };
    if !animator.starts_enabled() { return; }
    if mv.clip_phase == 0 {
        mv.clip_phase = 1;
        if let Some((name, looping)) = clip_for(mv.kind, false) {
            let hold = f64::from(mv.remaining());
            if log() {
                info!("skill motion: clip {name} for {hold:.2} s");
            }
            play(commands, entity, &mut animator, loco, clips, name, looping, hold, now);
        }
    }
    if mv.clip_phase == 1 && mv.landed {
        mv.clip_phase = 2;
        if mv.recover > 0.0
            && let Some((name, looping)) = clip_for(mv.kind, true)
        {
            play(commands, entity, &mut animator, loco, clips, name, looping, f64::from(mv.recover), now);
        }
    }
}

/// The local player: input is locked, the move plays, the server hears about it.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_local(
    time: Res<Time>,
    clips: Res<ClipLibrary>,
    terrain: Res<TerrainQuery>,
    collision: Res<WorldCollision>,
    mut lag: ResMut<CameraLag>,
    mut out: MessageWriter<NetCommand>,
    mut commands: Commands,
    mut players: Query<(Entity, &LocalPlayer, &mut Mover, &mut Transform, &mut SkillMove, &mut Motion, Option<&mut Animator>, Option<&Locomotion>, Has<super::seating::Seated>), Without<Dead>>,
    seats: Option<Res<super::seating::Seats>>,
) {
    let dt = time.delta_secs().min(0.1);
    let now = time.elapsed_secs_f64();
    for (entity, local, mut mover, mut transform, mut mv, mut motion, animator, loco, seated) in &mut players {
        if seated && !mv.controller_activity() {
            // Native parented Blink refuses translation and sends no unbond. A
            // bound impulse flight has no proven solver here: decline that
            // positional approximation while preserving real damage/buff poses.
            // Blink's separate native local-quaternion write remains unsupported.
            commands.entity(entity).try_remove::<SkillMove>();
            continue;
        }
        if mv.controller_activity() && seats.as_ref().is_some_and(|s| s.wanted(local.obj_id).is_some()) {
            // The existing captured unbond must complete before this app solver
            // and movement reports resume. This transport-latency hold is an
            // integration approximation, not the native global frame schedule.
            continue;
        }
        let before = mover.pos;
        let step = advance(&mut mv, mover.pos, dt, &terrain, &collision);
        start_clip(&mut mv, &mut commands, entity, animator, loco, &clips, now);
        mover.pos = step.pos;
        mover.velocity = step.velocity;
        mover.vz = step.velocity.z;
        mover.grounded = !step.airborne;
        mover.swim = None;
        if mv.rotate && !mv.landed && mv.flight.dist > 0.0 {
            let d = mv.flight.dir;
            mover.facing = (-d.x).atan2(d.y);
        }
        *transform = placement(mover.pos, mover.facing, 1.0);
        lag.push(mover.pos - before);
        *motion = Motion { speed: 0.0, local_dir: Vec2::ZERO, airborne: step.airborne && mv.flight.lift() > 0.4 };
        mv.sent += dt;
        let ending = step.done;
        if mv.sent >= SEND_INTERVAL || ending || mv.flight.eta == 0.0 {
            mv.sent = 0.0;
            let to_wire = |v: f32| (v * 1024.0).round().clamp(-32767.0, 32767.0) as i16;
            let flying = !ending && !mv.landed;
            let state = if !step.airborne { MoveState::Ground } else if step.velocity.z > 0.0 { MoveState::Jump } else { MoveState::Fall };
            let velocity = if flying { [to_wire(step.velocity.x), to_wire(step.velocity.y), to_wire(step.velocity.z)] } else { [0; 3] };
            out.write(NetCommand(Command::Move(
                MoveCommand {
                    pos: Pos::new(mover.pos.x, mover.pos.y, mover.pos.z),
                    yaw: mover.facing,
                    moving: flying,
                    stance: aa_net::model::stance::RELAXED,
                    velocity,
                    delta_movement: [0, if flying { 127 } else { 0 }, 0],
                    ..MoveCommand::default()
                }
                .with_state(state),
            )));
        }
        if ending {
            mover.velocity = Vec3::ZERO;
            mover.vz = 0.0;
            if log() {
                info!("skill motion: the player lands at ({:.1}, {:.1}, {:.2})", mover.pos.x, mover.pos.y, mover.pos.z);
            }
            commands.entity(entity).try_remove::<SkillMove>();
        }
    }
}

/// Other units: the flight is played over the unit's own track, which is reset to the landing place.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_remote(
    time: Res<Time>,
    clips: Res<ClipLibrary>,
    terrain: Res<TerrainQuery>,
    collision: Res<WorldCollision>,
    mut commands: Commands,
    mut units: Query<(Entity, &mut RemoteUnit, &mut Transform, &mut SkillMove, Option<&mut Animator>, Option<&Locomotion>, Has<super::seating::Seated>)>,
) {
    let dt = time.delta_secs().min(0.1);
    let now = time.elapsed_secs_f64();
    for (entity, mut unit, mut transform, mut mv, animator, loco, seated) in &mut units {
        if seated {
            // Remote controllers are development simulation; another client owns
            // live exit/movement. Do not invent an unbond for a foreign actor.
            commands.entity(entity).try_remove::<SkillMove>();
            continue;
        }
        let step = advance(&mut mv, unit.pos, dt, &terrain, &collision);
        start_clip(&mut mv, &mut commands, entity, animator, loco, &clips, now);
        unit.pos = step.pos;
        if mv.rotate && !mv.landed && mv.flight.dist > 0.0 {
            let d = mv.flight.dir;
            unit.facing = (-d.x).atan2(d.y);
        }
        *transform = placement(unit.pos, unit.facing, unit.scale());
        if step.done {
            let (pos, facing) = (unit.pos, unit.facing);
            unit.reset_track(pos, facing);
            commands.entity(entity).try_remove::<SkillMove>();
        }
    }
}

/// A made-up step of `AA_MOTION_FAKE`.
enum DevStep {
    /// Knocked back by the target unit (mm, degrees).
    Push(Knock),
    /// Blinked forward (mm), as `Blink` (Teleportation) does.
    Blink(f32),
    /// Blinked to just behind the target unit (mm), as `TeleportToUnit` (Shadow Step) does.
    Behind(f32),
}

/// Dev: `AA_MOTION_FAKE="T:push:DIST_MM:ANGLE;T:blink:DIST_MM;T:behind:DIST_MM"`: `T` seconds after the first frame the local player is
/// knocked back by the unit `AA_COMBAT_FAKE_TARGET` names (an object id, `model=ID` or a part of its name), blinks forward, or blinks
/// to behind that unit: what the hit of a `KnockBack` skill and the `SCBlinkUnit` of Teleportation / Shadow Step do, for the
/// skills the lane's server cannot cast for a test character.
fn dev_script(
    time: Res<Time>,
    book: Res<Book>,
    roster: Roster,
    mut tracker: ResMut<Tracker>,
    mut commands: Commands,
    mut script: Local<Option<(f64, Vec<(f64, DevStep)>)>>,
) {
    let Some((player, me, mover)) = roster.locals.iter().next().map(|(e, l, m)| (e, l.obj_id, m)) else { return };
    let now = time.elapsed_secs_f64();
    let (started, steps) = script.get_or_insert_with(|| {
        let steps = std::env::var("AA_MOTION_FAKE").ok().map(|text| {
            let mut steps: Vec<(f64, DevStep)> = text
                .split(';')
                .filter_map(|s| {
                    let mut p = s.split(':').map(str::trim);
                    let at: f64 = p.next()?.parse().ok()?;
                    let op = p.next()?;
                    let mm: f32 = p.next()?.parse().ok()?;
                    let angle: f32 = p.next().and_then(|a| a.parse().ok()).unwrap_or(0.0);
                    let step = match op {
                        "push" => DevStep::Push(Knock { distance: mm * 0.001, angle }),
                        "blink" => DevStep::Blink(mm * 0.001),
                        "behind" => DevStep::Behind(mm * 0.001),
                        _ => return None,
                    };
                    Some((at, step))
                })
                .collect();
            steps.sort_by(|a, b| b.0.total_cmp(&a.0));
            steps
        });
        (now, steps.unwrap_or_default())
    });
    if steps.is_empty() {
        return;
    }
    let wanted = std::env::var("AA_COMBAT_FAKE_TARGET").unwrap_or_default().to_lowercase();
    let other = roster
        .units
        .iter()
        .find(|u| u.obj_id != me && (u.obj_id.to_string() == wanted || wanted.strip_prefix("model=").is_some_and(|m| m == u.model_id.to_string()) || (!wanted.is_empty() && u.name.to_lowercase().contains(&wanted))))
        .map(|u| u.obj_id);
    while steps.last().is_some_and(|s| now - *started >= s.0) {
        let (_, step) = steps.pop().unwrap();
        let mut start = Starter { book: &book, roster: &roster, commands: &mut commands, tracker: &mut tracker, now };
        match step {
            DevStep::Push(knock) => start.knock(me, other.unwrap_or(0), None, knock),
            DevStep::Blink(distance) => {
                let f = path::forward(mover.facing);
                start.blink(player, mover.pos, mover.pos + Vec3::new(f.x * distance, f.y * distance, 0.0));
            }
            DevStep::Behind(distance) => {
                let Some(t) = other.and_then(|o| roster.point(o)) else { continue };
                let away = Vec2::new(t.x - mover.pos.x, t.y - mover.pos.y).normalize_or_zero();
                start.blink(player, mover.pos, t + Vec3::new(away.x * distance, away.y * distance, 0.0));
            }
        }
    }
}

fn decay_lag(time: Res<Time>, mut lag: ResMut<CameraLag>) {
    lag.decay(time.delta_secs());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_trails_a_move_and_catches_up() {
        let mut lag = CameraLag::default();
        // 8 m in one step: the pivot stays 7 m behind (the limit)
        lag.push(Vec3::new(0.0, 8.0, 0.0));
        assert!((lag.0.length() - LAG_MAX).abs() < 1e-4 && lag.0.y < 0.0);
        for _ in 0..60 {
            lag.decay(1.0 / 60.0);
        }
        assert!(lag.0.length() < 0.01, "{:?}", lag.0);
    }

    /// A charge over real terrain (the pond near the spawn): the steps are as fast as the path says, the unit never sinks under the
    /// ground and ends where the path ends, standing on it.
    #[test]
    fn a_charge_over_the_terrain_ends_on_the_ground_at_its_end_point() {
        let terrain = TerrainQuery::new("main_world");
        let collision = WorldCollision::new("main_world", terrain.clone());
        let (x, y) = (15578.2f32, 15368.1f32);
        let Some(z) = terrain.height_at(x, y) else { return };
        let from = Vec3::new(x, y, z);
        let target = Vec3::new(x, y + 10.0, terrain.height_at(x, y + 10.0).unwrap_or(z));
        let mut v = [0; 13];
        v[1] = 10;
        v[2] = 400;
        v[3] = -800;
        v[6] = 1;
        let c = Controller { id: 1, kind: 2, v, start_anim: 0 };
        let end = path::leap_end(&c, from, 0.0, Some(target)).unwrap();
        let mut mv = SkillMove::new(path::leap(&c, from, end, 1.0).unwrap(), Kind::Dash, true);
        let mut pos = from;
        let mut frames = 0;
        loop {
            let step = advance(&mut mv, pos, 1.0 / 60.0, &terrain, &collision);
            assert!((step.pos - pos).length() < 0.6, "no frame jumps: {}", (step.pos - pos).length());
            if let Some(g) = terrain.height_at(step.pos.x, step.pos.y) {
                assert!(step.pos.z >= g - 0.02, "under the ground: {} < {g}", step.pos.z);
            }
            pos = step.pos;
            frames += 1;
            if step.done || frames > 120 {
                break;
            }
        }
        assert!(frames <= 30, "{frames} frames for 0.4 s");
        assert!(Vec2::new(pos.x - end.x, pos.y - end.y).length() < 0.3, "{pos} vs {end}");
        assert!(mv.landed);
    }

    #[test]
    fn only_the_events_that_move_units_are_queued() {
        let mut inbox = Inbox::default();
        observe(&mut inbox, &Event::UnitDeath { obj_id: 3 }, 1.0);
        observe(&mut inbox, &Event::PlotEnded { tl: 3 }, 1.0);
        assert_eq!(inbox.0.len(), 1);
    }
}

#[cfg(test)]
#[path = "skill_motion/seating_tests.rs"]
mod seating_tests;
