//! Boarding and leaving a cart or other slave unit, with the original client's checks.
//!
//! A right click on a slave (`UnitKind::Slave`, unit kind byte 2) sends `CSChangeTarget` 0x02c
//! (when the slave is not the target yet) and `CSStartInteraction` 0x068 with `extra_info` 1 and
//! mouse button 3 (`FUN_390f3780` -> `FUN_393be030`; live capture 2026-10-28). It never sends
//! `CSBindSlave` 0x031 directly: the server answers the 0x068 with `SCNpcInteractionSkillList`
//! 0x07b, whose FIRST skill is acted on at once (`FUN_393be850` -> `FUN_393be280`). The slave
//! bind itself is the "occupy" skill 12076 (`FUN_393bf310` -> `FUN_39362f50` -> `FUN_393464a0`),
//! after the reach check of `FUN_393bf270` / `FUN_39093860`: `DAT_39ac79fc` 9.0, or
//! `DAT_39ac7a00` 12.25 when the player unit's +0x3c26 mounted byte is set, square-rooted to
//! 3.0 / 3.5 m and compared strictly (`gap < limit`) against the 3D SURFACE gap between the
//! collision shapes (`FUN_3989bf90` -> `FUN_398b1ab0` -> `FUN_398b1af0`: the minimum over the
//! target's colliders of `|centre_a - centre_b| - (r_a + r_b)`, not clamped at 0). Then the
//! local checks of `FUN_39346290` / `FUN_393464a0`: the slave is dead (0x144), our own unit is
//! dead (`CANT_DO_WHILE_YOU_DEAD` 0x14c), we are bound to another slave (0x66), we are bound to
//! this one (silent), a bind is already pending (0x13), and the same gap strictly below
//! `DAT_39ac7408` 16.0 -> 4.0 m (0x9a). 1.2 minimises over per-collider shapes that are not
//! recovered; a slave (a `VehicleModel`) has no `ActorModel` radius, so this client measures the
//! player to the drawn body's prefab box ([`slave_gap`]: the body only, without the wheels the
//! summon placement's box adds, in the slave's yaw frame), not to its origin. That box is an approximation of the collision
//! parts, like the server's ship mass box / attach-point stand-in.
//! The auto-walk out of reach (`FUN_390a6af0`) is not modelled: it refuses 0x9a.
//!
//! A right click on the slave we are bound to or seated on sends only `CSChangeTarget` 0x02c
//! (live capture 2026-10-28, case e), never 0x068.
//!
//! Skills 13863 (passenger) goes to `CSStartSkill` in 1.2; 19165 (destroy) needs the same
//! generic interaction cast but is not ported. The passenger request targets the slave, not the
//! player, and the server attaches point 2 (`AttachTo` value1 2). A second seat needs leaving the
//! first: there is no change-seat packet, and this client does not refuse a passenger request
//! while already seated; the server decides (capture-pending S11-C2). Any other first skill (0 or
//! unknown) does nothing.
//!
//! The interactive key (F) on a slave is the dynamic action bar's 12076 entry (`FUN_393b1de0` ->
//! `FUN_393be280`, through `candidate` below); NPC talk keeps the path it had. The player flags
//! at +0x3a70..+0x3a73/+0x3a7b (native 0x13) and the parent unit type 0xe (native silent refusal)
//! are not ported: their meaning is unverified.
//!
//! Leaving is `X2SiegeWeapon:DiscardSiegeWeapon` (`FUN_394aa090` -> `FUN_39345aa0`):
//! `CSDiscardSlave` 0x032 for the slave `SCSlaveBound` named as the local player's master (its
//! timeline, `this+0x14`). The native refuses with 0x67 (`SLAVE_SHUT_OFF_TO_UNBIND`) only when
//! BOTH hold: the bound slave model's +0x2a0 slot is non-zero, and the model's entity velocity
//! squared is above 25.0 (`_DAT_3999f85c`, 5 m/s) (`FUN_39345aa0` 677646-677660,
//! `FUN_39345970` 677552-677561). For a `VehicleModel` the slot is
//! `use_wheeled_vehicle_simulation` (`X2::VehicleUnitModel::Init`, `FUN_390f0510` 190001, copies
//! the model descriptor's +0x72); for a `ShipModel` it holds the ship's 0x198-byte driver object
//! pointer (`FUN_390bdf70` 156143-156149), so a ship driver is refused too (inferred: a ship
//! dismount at speed is capture-pending). The 2026-11-01 owner capture: the Red Farm Hauler
//! (slave template 146, model 1553, `vehicle_models` 65: `use_wheeled_vehicle_simulation`
//! false) sent 0x032 at ~6.7 m/s and the owner got off, with no 0x67.
//!
//! A passenger (attach point above 1) leaves through the same explicit dismount path
//! (`FUN_39362170`), throttled to one request per 1500 ms (`_DAT_3a169948`); that branch has no
//! speed check. The 2026-11-02 12:30 real 1.2 capture (owner, Red Farm Hauler passenger via G)
//! pressed W and the client sent nothing (no CSMoveUnit, no CSDiscardSlave); the 12:30:06
//! CSDiscardSlave came from the R dismount skill. So 1.2 keeps a passenger seated on W, and the
//! decompile `FUN_39406540` -> `FUN_39404d50` -> `FUN_39362170` path must be blocked for
//! passengers by one of its guards (unresolved which). This client keeps the seat.

use std::time::Duration;

use aa_net::interaction::{Interact, InteractEvent};
use aa_net::model::{Unit, UnitKind};
use aa_net::slave_control::SlaveCommand;
use aa_net::state::Units;
use aa_net::{Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};
use web_time::Instant;

use super::alerts;
use super::api::{HudBridge, HudData, Request, Shared};

/// `DAT_39ac79fc` (9.0): `FUN_393bf270`'s slave interaction reach, sqrt(9.0) = 3.0 m, compared
/// strictly (`FUN_39093860`: `gap < limit`) against the 3D surface gap.
pub(super) const INTERACT_REACH: f32 = 3.0;
/// `DAT_39ac7a00` (12.25): the same reach while the player unit's +0x3c26 mounted byte is set,
/// sqrt(12.25) = 3.5 m.
pub(super) const MOUNTED_INTERACT_REACH: f32 = 3.5;
/// `DAT_39ac7408` (16.0): `FUN_39346290`'s bind reach, sqrt(16.0) = 4.0 m.
pub(super) const BIND_REACH: f32 = 4.0;
/// `_DAT_399d3230` (6.25): the interaction indicator's candidate reach, sqrt(6.25) = 2.5 m.
/// The native passes the squared value to `FUN_393601a0`, which compares the surface gap with
/// `sqrt` of it (`FUN_39093860`).
pub(super) const CANDIDATE_REACH: f32 = 2.5;
/// A slave farther than this centre distance from the player cannot have a body box reaching
/// the candidate gap: the longest 1.2 ship body is well under 32 m. Skip it before
/// [`slave_gap`] loads the model from the pak (the F prompt producer runs every frame).
const SLAVE_PREFILTER: f32 = 64.0;
/// `_DAT_3999f85c` (25.0): the driver's unbind speed limit, 5 m/s, checked only when the bound
/// slave model's +0x2a0 slot is non-zero (a wheeled-simulation vehicle, or a ship).
pub(super) const UNBIND_SPEED_SQUARED: f32 = 25.0;
/// `_DAT_3a169948`: the passenger dismount throttle.
pub(super) const WALK_OFF_THROTTLE: Duration = Duration::from_millis(1500);

/// `ErrorMessageType` ids from the native message table (`FUN_3956b860`).
const MSG_ALREADY_REQUESTED: i16 = 0x13;
const MSG_SLAVE_UNBIND_FIRST: i16 = 0x66;
const MSG_SLAVE_SHUT_OFF_TO_UNBIND: i16 = 0x67;
const MSG_TOO_FAR_AWAY: i16 = 0x9a;
const MSG_SLAVE_CANNOT_BIND_WHILE_IS_DEAD: i16 = 0x144;
const MSG_CANT_DO_WHILE_YOU_DEAD: i16 = 0x14c;

/// What a boarding or leaving request did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Queued as `Request::Slave`.
    Sent,
    /// Refused without a message (already bound to this slave).
    Silent,
    /// Refused with this `ErrorMessageType`.
    Refused(i16),
}

/// The slave a right click picked. The interact key (F) sends nothing for a slave: the 1.2
/// capture with a cart targeted produced no packet, while plain NPC talk keeps its own path.
pub(super) fn action_target(units: &Units, picked: Option<u32>) -> Option<u32> {
    picked.filter(|id| {
        units
            .get(*id)
            .is_some_and(|unit| matches!(unit.kind, UnitKind::Slave { .. }))
    })
}

/// The reach predicate of `FUN_3989bf90` -> `FUN_398b1ab0` -> `FUN_398b1af0`: the 3D surface
/// gap of two sphere colliders, `|centre_a - centre_b| - (r_a + r_b)`, not clamped at 0. 1.2
/// takes the minimum over the target's several colliders; this port uses one sphere per unit
/// (unverified). A slave has no `ActorModel`, so [`radius`] returns 0 for it and its gap is the
/// centre distance minus the player's actor radius.
pub(super) fn surface_gap(from: Vec3, from_radius: f32, to: Vec3, to_radius: f32) -> f32 {
    from.distance(to) - (from_radius + to_radius)
}

/// The key `FUN_393601a0` (units) and `FUN_3902f010` (doodads) store per candidate, ordered by
/// `FUN_3902ed20`: the surface gap minus `1 + facing` (`_DAT_3999c488` is the 1.0), where
/// `facing` is the player's forward dot the horizontal direction to the candidate. The gap is
/// taken absolute like the native bits mask (`DAT_3999d500` 0x7fffffff); lower sorts first. The
/// F-bar producer additionally clamps a slave's signed gap at 0 before this key; the
/// bind/right-click checks keep the signed value.
pub(super) fn native_key(from: Vec3, forward: Vec2, at: Vec3, gap: f32) -> f32 {
    let toward = (at - from).truncate().normalize_or_zero();
    gap.abs() - (1.0 + forward.dot(toward))
}

/// The target's scaled actor-model radius, the same source as `api::distance_to`. A slave's
/// model is a `VehicleModel`, not an `ActorModel`, so the catalog has no radius for it and this
/// returns 0; [`slave_gap`] uses the model's box instead.
fn radius(data: &HudData, unit: &Unit) -> f32 {
    data.game
        .get()
        .map_or(0.0, |game| game.model_size(unit.model_id, unit.scale))
}

/// The surface gap to a slave: the player's distance to the vehicle body's box in the slave's
/// frame, minus the player's radius. The 1.2 client measures the player's collision shape to the
/// target's collision parts (`FUN_3989bf90` -> `FUN_398b1ab0`); those shapes are not recovered,
/// so the prefab body box [`crate::ingame::data::Db::vehicle_look`] draws and the summon
/// placement uses stands in for them, like the server's ship mass box / attach-point measure.
/// The box is authored at scale 1 in Cry model space (Z up); it is rotated by the slave's yaw and
/// divided by the unit scale before the clamp. A model with no loaded box keeps the old centre
/// distance minus the (zero for a vehicle) actor radius.
pub(super) fn slave_gap(
    data: &HudData,
    unit: &Unit,
    from: Vec3,
    at: Vec3,
    yaw: f32,
) -> f32 {
    let Some((lo, hi)) = data
        .game
        .get()
        .and_then(|game| game.vehicle_box(unit.model_id))
    else {
        return surface_gap(from, data.player_radius, at, radius(data, unit));
    };
    let scale = if unit.scale > 0.0 { unit.scale } else { 1.0 };
    let local = Quat::from_rotation_z(-yaw) * (from - at) / scale;
    let nearest = local.clamp(lo, hi);
    // `local` is in unscaled model units: put the distance back into metres before subtracting
    // the player's world-space radius.
    local.distance(nearest) * scale - data.player_radius
}

/// Where the checks measure from. While attached to a slave the server moves only the slave,
/// and the HUD position (the local entity's transform) can stay at the boarding point: a
/// point-1 seat marks the cart `Driven`, so the client drives the cart from the stale mover
/// instead of following it, and a cart has no `Animator` for the seat pose. Use the attached
/// unit's replicated position then.
pub(super) fn player_centre(data: &HudData, world: &impl Fn(u32) -> Option<(Vec3, f32)>) -> Vec3 {
    data.mounted
        .player_attachment()
        .and_then(|attachment| world(attachment.obj_id))
        .map_or(data.pos, |(pos, _)| pos)
}

/// The facing the candidate checks use. While seated the HUD yaw can stay at the boarding
/// point (the same staleness as [`player_centre`]), so use the attached unit's world yaw (yaw 0
/// faces +y).
pub(super) fn player_forward(data: &HudData, world: &impl Fn(u32) -> Option<(Vec3, f32)>) -> Vec2 {
    let yaw = data
        .mounted
        .player_attachment()
        .and_then(|attachment| world(attachment.obj_id))
        .map_or(data.yaw, |(_, yaw)| yaw);
    Vec2::new(-yaw.sin(), yaw.cos())
}

/// A unit's world pose from the packets alone (a child slave composed through its parent), for
/// the callers that have no drawn transforms at hand.
pub(super) fn replicated_pose(units: &Units) -> impl Fn(u32) -> Option<(Vec3, f32)> + '_ {
    move |id| {
        units
            .get(id)
            .and_then(|unit| crate::ingame::units::unit_world_pose_in(unit, units))
    }
}

/// `FUN_393bf270`: the interaction reach in metres (3.5 on a mount). Only riding a mate counts as
/// mounted here; whether a slave seat also sets +0x3c26 is unverified.
fn interact_reach(data: &HudData) -> f32 {
    if data.mounted.riding_mate() {
        MOUNTED_INTERACT_REACH
    } else {
        INTERACT_REACH
    }
}

/// `FUN_390f3780` -> `FUN_393be030`: the right-click interaction request. The captured body for
/// cart bc 0x5f3c is `3c 5f 00 00 00 00 01 00 00 00 00 00 00 00 03 00 00 00 00`
/// (`npc_obj` cart, `obj` 0, `extra_info` 1, `pick_id` 0, mouse 3, modifiers 0).
///
/// While bound to or seated on this slave a right click only changes the target (live capture
/// 2026-10-28, case e: 0x02c, no 0x068, no message).
pub(super) fn interact(
    data: &mut HudData,
    units: &Units,
    target: Option<u32>,
    obj_id: u32,
) -> Outcome {
    let retarget = target != Some(obj_id);
    if retarget {
        data.requests.push(Request::Target { obj_id });
    }
    if current_slave(data, units) == Some(obj_id) {
        return if retarget {
            Outcome::Sent
        } else {
            Outcome::Silent
        };
    }
    data.requests.push(Request::Interact(Interact::Start {
        npc_obj_id: obj_id,
        obj_id: 0,
        extra_info: 1,
        pick_id: 0,
        mouse_button: 3,
        modifier_keys: 0,
    }));
    Outcome::Sent
}

/// `FUN_393be850` / `FUN_393be280`: `SCNpcInteractionSkillList` 0x07b for a slave acts on the
/// first skill at once. The two skills the 1.2 tables name for a slave:
/// * 12076 "occupy" -> `CSBindSlave` 0x031 (the driver seat, point 1);
/// * 13863 "passenger seat" -> a plain `CSStartSkill` 0x052 with the slave as the unit target
///   (`AttachTo` value1 2 attaches point 2).
/// 19165 (destroy) and any other skill do nothing here.
const SKILL_OCCUPY: u32 = 12076;
const SKILL_PASSENGER: u32 = 13863;

/// Which seat a slave interaction skill asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SlaveSeat {
    Driver,
    Passenger,
}

impl SlaveSeat {
    fn of_skill(skill: u32) -> Option<SlaveSeat> {
        match skill {
            SKILL_OCCUPY => Some(SlaveSeat::Driver),
            SKILL_PASSENGER => Some(SlaveSeat::Passenger),
            _ => None,
        }
    }
}

/// The slave the interaction indicator would offer, and the skills the dynamic action bar shows.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Candidate {
    pub obj_id: u32,
    pub template: u32,
    /// 12076 always; 13863 when the model has a free `$passenger0` point.
    pub skills: Vec<u32>,
    /// The native ordering key (see [`native_key`]): lower sorts first, shared with the doodad
    /// producer so the dynamic action bar arbitrates both on the same value.
    pub key: f32,
    /// Centre distance, metres (the HUD's NPC-talk priority uses it).
    pub dist: f32,
}

/// `InteractionIndicator::MakeCandidate` (`FUN_393bdad0`) for a slave (`FUN_393bbb80`): every
/// slave within `_DAT_399d3230` 6.25 -> 2.5 m surface gap is offered, ordered by the native key
/// `gap - (1 + facing)` (`FUN_393601a0`; `FUN_3902ed20` sorts the one unit/doodad set by it).
/// No candidate while riding a mate (`player+0x3c26`, the only writer of that byte is the mate
/// mount path), while sitting on a doodad (`player+0x3b94`, set by `FUN_393635e0`), and none for
/// the slave already bound to or sat on (`FUN_3901be50` / `FUN_3901c5f0`); another slave is
/// still offered. The native builder has no dead check, so neither does this.
///
/// The facing is only the key's bonus, not a gate: a slave behind the player is still offered
/// when it is the only one in reach.
pub(super) fn candidate(data: &HudData, units: &Units) -> Option<Candidate> {
    if data.mounted.riding_mate() || data.doodad.seated {
        return None;
    }
    let game = data.game.get()?;
    let world = replicated_pose(units);
    let from = player_centre(data, &world);
    let forward = player_forward(data, &world);
    let seated = current_slave(data, units);
    let mut best: Option<(f32, Candidate)> = None;
    for unit in units.iter() {
        let UnitKind::Slave { template_id, .. } = unit.kind else {
            continue;
        };
        if seated == Some(unit.obj_id) {
            continue;
        }
        let (at, yaw) = world(unit.obj_id)
            .unwrap_or((Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z), unit.yaw()));
        if from.distance(at) > CANDIDATE_REACH + SLAVE_PREFILTER {
            continue;
        }
        // The bar orders a unit and a doodad on the same native key. A player standing inside
        // the body box has a negative surface gap; clamping it at 0 keeps the key from beating
        // every doodad the player faces. The bind/right-click checks below keep the raw gap.
        let gap = slave_gap(data, unit, from, at, yaw).max(0.0);
        if gap >= CANDIDATE_REACH {
            continue;
        }
        let mut skills = vec![SKILL_OCCUPY];
        if game.mode_actions.has_passenger_point(template_id)
            && !units.iter().any(|u| {
                u.attached_to
                    .is_some_and(|a| a.obj_id == unit.obj_id && a.point == 2)
            })
        {
            skills.push(SKILL_PASSENGER);
        }
        let key = native_key(from, forward, at, gap);
        let candidate = Candidate {
            obj_id: unit.obj_id,
            template: template_id,
            skills,
            key,
            dist: from.distance(at),
        };
        if best.as_ref().is_none_or(|(best, _)| key < *best) {
            best = Some((key, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

pub(crate) fn receive(
    data: &mut HudData,
    units: &Units,
    event: &Event,
    world: &impl Fn(u32) -> Option<(Vec3, f32)>,
) -> Outcome {
    let Event::Interaction(InteractEvent::SkillList {
        npc_obj_id, skills, ..
    }) = event
    else {
        return Outcome::Silent;
    };
    let Some(unit) = units.get(*npc_obj_id) else {
        return Outcome::Silent;
    };
    let Some(seat) = skills.first().copied().and_then(SlaveSeat::of_skill) else {
        return Outcome::Silent;
    };
    if !matches!(unit.kind, UnitKind::Slave { .. }) {
        return Outcome::Silent;
    }
    let (at, yaw) = world(unit.obj_id)
        .unwrap_or((Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z), unit.yaw()));
    let gap = slave_gap(data, unit, player_centre(data, world), at, yaw);
    if gap >= interact_reach(data) {
        return Outcome::Refused(MSG_TOO_FAR_AWAY);
    }
    match seat {
        SlaveSeat::Driver => bind(data, units, *npc_obj_id, world),
        SlaveSeat::Passenger => passenger(data, units, *npc_obj_id),
    }
}

/// The passenger seat request: `CSStartSkill` 13863 with the slave as the unit target. The
/// original client goes from the interaction range check straight to the skill
/// (`FUN_393bf310` -> `FUN_39362f50`); the bind checks of `FUN_39346290` belong to 12076.
/// `FUN_39362f50` refuses a dead target with 0x144 for 13863/19165 and has no seated refusal, so
/// this port keeps only the dead check. A right click while seated never reaches this path: it
/// sends only `CSChangeTarget` (`interact`). Riding a mate only widens the interaction reach; the
/// port does not refuse it (capture-pending S11-C5).
pub(super) fn passenger(data: &mut HudData, units: &Units, target: u32) -> Outcome {
    if units.get(target).is_some_and(|unit| unit.dead) {
        return Outcome::Refused(MSG_SLAVE_CANNOT_BIND_WHILE_IS_DEAD);
    }
    data.requests.push(Request::UseSkill {
        skill_id: SKILL_PASSENGER,
        target_obj_id: target,
    });
    Outcome::Sent
}

/// Feeds `SCNpcInteractionSkillList` 0x07b for slaves to [`receive`].
pub(super) fn track(
    mut events: MessageReader<NetEvent>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    views: Option<Res<crate::ingame::units::Views>>,
    transforms: Query<&Transform>,
    parents: Query<&ChildOf>,
    rt: Option<NonSend<UiRuntime>>,
) {
    let world = |id: u32| {
        let unit = units.get(id)?;
        match views.as_deref() {
            Some(views) => {
                crate::ingame::units::unit_world_pose(unit, &units, views, &transforms, &parents)
            }
            None => crate::ingame::units::unit_world_pose_in(unit, &units),
        }
    };
    for NetEvent(event) in events.read() {
        let outcome = receive(&mut bridge.0.borrow_mut(), &units, event, &world);
        if let Outcome::Refused(code) = outcome
            && let Some(rt) = rt.as_deref()
        {
            announce(rt, code);
        }
    }
}

/// The slave we are on or bound to: a received attachment wins over the SCSlaveBound fallback.
fn current_slave(data: &HudData, units: &Units) -> Option<u32> {
    if let Some(attachment) = data.mounted.player_attachment()
        && units
            .get(attachment.obj_id)
            .is_some_and(|unit| matches!(unit.kind, UnitKind::Slave { .. }))
    {
        return Some(attachment.obj_id);
    }
    data.mounted.bound_slave().map(|(obj_id, _)| obj_id)
}

/// `FUN_39346290` / `FUN_393464a0`: the client's checks before `CSBindSlave`.
pub(super) fn bind(
    data: &mut HudData,
    units: &Units,
    target: u32,
    world: &impl Fn(u32) -> Option<(Vec3, f32)>,
) -> Outcome {
    let Some(unit) = units.get(target) else {
        return Outcome::Silent;
    };
    let UnitKind::Slave { tl_id, .. } = unit.kind else {
        return Outcome::Silent;
    };
    if unit.dead {
        return Outcome::Refused(MSG_SLAVE_CANNOT_BIND_WHILE_IS_DEAD);
    }
    // FUN_39346290 shows a dead player through FUN_39390900(2), a different message table;
    // 0x14c is the proven `CANT_DO_WHILE_YOU_DEAD` text of the interact path.
    if data.mounted.player_dead() {
        return Outcome::Refused(MSG_CANT_DO_WHILE_YOU_DEAD);
    }
    if let Some(bound) = current_slave(data, units) {
        if bound != target {
            return Outcome::Refused(MSG_SLAVE_UNBIND_FIRST);
        }
        return Outcome::Silent;
    }
    if data.mounted.bind_pending(Instant::now()) {
        return Outcome::Refused(MSG_ALREADY_REQUESTED);
    }
    let (at, yaw) = world(unit.obj_id)
        .unwrap_or((Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z), unit.yaw()));
    let gap = slave_gap(data, unit, player_centre(data, world), at, yaw);
    if gap >= BIND_REACH {
        return Outcome::Refused(MSG_TOO_FAR_AWAY);
    }
    // Not a 1.2 value: a zero timeline is not sendable, so stay silent like the same-slave case.
    if tl_id == 0 {
        return Outcome::Silent;
    }
    data.mounted.note_bind(target, tl_id);
    data.requests
        .push(Request::Slave(SlaveCommand::Bind { tl_id }));
    Outcome::Sent
}

/// `FUN_39345aa0` through `X2SiegeWeapon:DiscardSiegeWeapon` and the mode bar's `unbind` slot:
/// `CSDiscardSlave` for the slave `SCSlaveBound` named as the local player's master. That same
/// bound slave is the one speed-checked (refused above 5 m/s with 0x67 on a gated model), even
/// when the player is seated on another one. Without it (a passenger, or a point-1 seat that is not the player's)
/// the native `FUN_39362170` branch sends only from an attachment above point 1: no speed check,
/// one request per 1500 ms (`_DAT_3a169948`). The 2026-11-02 12:30 1.2 capture showed a
/// passenger's W sends nothing; the decompile input path is blocked for passengers by one of its
/// guards (unresolved), so this client does not wire movement to it.
pub(super) fn discard(data: &mut HudData) -> Outcome {
    discard_at(data, Instant::now())
}

pub(super) fn discard_at(data: &mut HudData, now: Instant) -> Outcome {
    // `FUN_39345aa0` takes its bound branch on `this+0x14`, the timeline SCSlaveBound stored for
    // the local player's master (`this+0x18` is that slave's object id, used for the speed
    // lookup); the seat point plays no part. Both the check and the packet use the bound slave,
    // never the seat's `discard_slave` preference.
    if let Some((obj_id, tl_id)) = data.mounted.bound_slave() {
        // `FUN_39345aa0` 677646 puts the speed check inside `if (model[+0x2a0] != 0)`: the
        // vehicle `use_wheeled_vehicle_simulation` flag (`FUN_390f0510` 190001) or a ship's
        // 0x198-byte ship object pointer (`FUN_390bdf70` 156143-156149, inferred). A land
        // vehicle such as the captured hauler has 0 and never refuses. A model the game data
        // does not name is not gated; the server still decides the unbind.
        let speed_gated = data.mounted.slave_model(obj_id).is_some_and(|model| {
            data.game
                .get()
                .is_some_and(|game| game.unbind_speed_gated(model))
        });
        // No replicated movement, or one older than the staleness bound, means a standing slave.
        if speed_gated
            && data.mounted.speed_squared(obj_id, now).unwrap_or(0.0) > UNBIND_SPEED_SQUARED
        {
            return Outcome::Refused(MSG_SLAVE_SHUT_OFF_TO_UNBIND);
        }
        data.requests
            .push(Request::Slave(SlaveCommand::Unbind { tl_id }));
        return Outcome::Sent;
    }
    let Some((_, tl_id)) = data.mounted.discard_slave() else {
        return Outcome::Silent;
    };
    if !data
        .mounted
        .player_attachment()
        .is_some_and(|attachment| attachment.point > 1)
    {
        return Outcome::Silent;
    }
    if !data.mounted.walk_off_throttle(now) {
        // Native FUN_39362170 returns silently inside the 1500 ms window.
        return Outcome::Silent;
    }
    data.requests
        .push(Request::Slave(SlaveCommand::Unbind { tl_id }));
    Outcome::Sent
}

/// The message of a refusal as the scripts show one (`ERROR_MSG` -> `SYSMSG`).
fn text(lua: &Lua, code: i16) -> String {
    let name = alerts::error_name(code).unwrap_or("UNKNOWN");
    alerts::localized_lua(lua, "ERROR_MSG", name).unwrap_or_else(|| name.to_owned())
}

/// Shows a refusal through the original system-message window.
pub(super) fn announce(rt: &UiRuntime, code: i16) {
    let text = text(&rt.lua, code);
    info!("hud: slave refusal {code:#x}: {text}");
    rt.fire_event("SYSMSG", text);
}

fn announce_lua(lua: &Lua, code: i16) -> mlua::Result<()> {
    let text = text(lua, code);
    info!("hud: slave refusal {code:#x}: {text}");
    aa_ui::lua::fire_ui_event(
        lua,
        "SYSMSG",
        vec![Value::String(lua.create_string(&text)?)],
    );
    Ok(())
}

/// `X2SiegeWeapon:DiscardSiegeWeapon()` (native `394aa090`).
pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let namespace: Table = lua.globals().get("X2SiegeWeapon")?;
    let shared = state.clone();
    namespace.set(
        "DiscardSiegeWeapon",
        lua.create_function(move |lua, _: MultiValue| {
            let outcome = discard(&mut shared.borrow_mut());
            if let Outcome::Refused(code) = outcome {
                announce_lua(lua, code)?;
            }
            Ok(())
        })?,
    )
}

#[cfg(test)]
mod tests;
