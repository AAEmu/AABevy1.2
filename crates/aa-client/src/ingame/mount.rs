//! Riders: a unit the server attached to a mount (`SCUnitAttached`: a mate such as a horse or a lion) sits on the mount's seat
//! bone and plays the rider's clips of that kind of mount.
//!
//! The server moves a rider with its mount: it sends no movement for the rider, only for the mount. The rider's clips come
//! from its character `.cal` (`horse_`, `lion_`, `wildboar_` and `pangolin_` sets; the `models.mount_pose_id` of the mount
//! says which) and follow the original player graph (`player_full_body.xml`): the reason's get-on one-shot, the
//! `DriverIdle` / `DriverWalk` / `DriverRun` loops and their idle-to-run / run-to-idle one-shots, then the get-off chain.
//! The mount's clips are its own `.cal` as for any creature. The seat is the mount's bone `bone_spine_driver` (attach point
//! 1) or `bone_spine_passenger`.
//!
//! The local player rides the same way ([`LocalMount`]). On the driver's seat of its own mate its keys move the mount:
//! `player::control_player` steers the [`Mover`] at the mount's speed and reports it as the mate's movement (`CSMoveUnit`
//! with the mate's object id; the server ignores a riding character's own movement), and [`drive_mount`] puts the mate's
//! unit where the mover is.

use aa_anim::pose::{BoundClip, Rig};
use aa_anim::{Animator, Cal, Character, ClipLibrary};
use aa_net::model::{Attachment, Equipment, ModelParams, Movement, UnitKind};
use aa_net::state::{Player, Units};
use aa_net::{Event, NetEvent, Pos};
use aa_world::coords::bevy_to_cry;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion, bind_clip};
use super::camera::OrbitCamera;
use super::data::{Db, MountSpeed};
use super::player::{LocalPlayer, Mover, SendState, placement};
use super::units::{RemoteUnit, Views};

/// The rider states of `player_full_body.xml` (`Horse_*` :1460, `Lion_*` :1601, `Bear_*` :1285, `Wildboar_*` :1839,
/// `Pangolin_*` :1684, `Elephant_*` :1383), picked by `models.mount_pose_id` (`FUN_39153b60`: 1 horse, 2 lion,
/// 3 bear, 4 wildboar, 9 elephant, 10 pangolin). Pose 0 (the donkeys, the elks, the yatas) takes the horse's clips;
/// the boat and car poses have no rider states and keep the old loop-only rider.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MountKind {
    Horse,
    Lion,
    Bear,
    Wildboar,
    Pangolin,
    Elephant,
}

/// Whether a pose has the graph's rider states. The boat and car poses (5 submarine, 6 smallboat, 7 coupleduckship,
/// 8 speedcar) have no `Enter`/`Exit` states in `player_full_body.xml`; they keep the rider loops of the old code.
fn rider_states(pose: u32) -> bool {
    !(5..=8).contains(&pose)
}

impl MountKind {
    fn of(pose: u32) -> MountKind {
        match pose {
            2 => MountKind::Lion,
            3 => MountKind::Bear,
            4 => MountKind::Wildboar,
            9 => MountKind::Elephant,
            10 => MountKind::Pangolin,
            _ => MountKind::Horse,
        }
    }

    /// The `*_DriverIdle` clip (`Horse_DriverIdle` :1464, `Lion_DriverIdle` :1601, ...).
    fn idle(self) -> &'static str {
        match self {
            MountKind::Lion | MountKind::Wildboar => "lion_ba_relaxed_idle",
            _ => "horse_ba_relaxed_idle",
        }
    }

    /// The `*_DriverWalk` clip (`Horse_DriverWalk` :1490, `Lion_DriverWalk` :1626, ...).
    fn walk(self) -> &'static str {
        match self {
            MountKind::Lion | MountKind::Wildboar => "lion_mo_normal_walk_f",
            _ => "horse_mo_normal_walk_f",
        }
    }

    /// The `*_DriverRun` clip, a `.cal` locomotion group (`_FIST_MO_RIDER_HORSE_RUN`, :1479).
    fn run(self) -> &'static str {
        match self {
            MountKind::Lion | MountKind::Bear => "_FIST_MO_RIDER_LION_RUN",
            MountKind::Wildboar => "_FIST_MO_RIDER_WILDBOAR_RUN",
            MountKind::Pangolin => "_FIST_MO_RIDER_PANGOLIN_RUN",
            MountKind::Horse | MountKind::Elephant => "_FIST_MO_RIDER_HORSE_RUN",
        }
    }

    /// The forward clip of [`MountKind::run`] when the `.cal` cannot resolve the group.
    fn run_fallback(self) -> &'static str {
        match self {
            MountKind::Lion | MountKind::Bear => "lion_mo_normal_run_f",
            MountKind::Wildboar => "wildboar_mo_normal_run_f",
            MountKind::Pangolin => "pangolin_mo_normal_run_f",
            MountKind::Horse | MountKind::Elephant => "horse_mo_normal_run_f",
        }
    }

    /// The `*_DriverIdleToRun` clip (`Horse_DriverIdleToRun` :1473, `Lion_DriverIdleToRun` :1609, ...). The
    /// `Elephant_*` group (:1383-1427) has no such state drawn in it, but the graph's lowercase
    /// `elephant_DriverIdleToRun` (:5266) is reachable through `Elephant_DriverIdle` ->
    /// `Elephant_DriverIdles_NullNode` -> `elephant_DriverIdleToRun` (:14470, :14476), and
    /// `elephant_DriverRunToIdle` (:5269) through `Elephant_DriverRun` (:14479); the elephant uses its own names.
    fn idle_to_run(self) -> &'static str {
        match self {
            MountKind::Lion | MountKind::Bear => "_FIST_MO_RIDER_LION_IDLETORUN",
            MountKind::Wildboar => "_FIST_MO_RIDER_WILDBOAR_IDLETORUN",
            MountKind::Elephant => "_FIST_MO_RIDER_ELEPHANT_IDLETORUN",
            MountKind::Horse | MountKind::Pangolin => "_FIST_MO_RIDER_HORSE_IDLETORUN",
        }
    }

    /// The forward clip when the `.cal` has no group for [`MountKind::idle_to_run`]. The elephant has none: its own
    /// state (see [`MountKind::run_to_idle`]) plays the misspelt `elehpant/` entry the 1.2 `.cal` names, which does not
    /// load, so it crossfades directly rather than play another mount's clip.
    fn idle_to_run_fallback(self) -> Option<&'static str> {
        match self {
            MountKind::Lion | MountKind::Bear => Some("lion_mo_relaxed_idletorun_f"),
            MountKind::Wildboar => Some("wildboar_mo_relaxed_idletorun_f"),
            MountKind::Horse | MountKind::Pangolin => Some("horse_mo_relaxed_idletorun_f"),
            MountKind::Elephant => None,
        }
    }

    /// The `*_DriverRunToIdle` clip (`Horse_DriverRunToIdle` :1487, `Lion_DriverRunToIdle` :1623, ...). The elephant's
    /// own state (`elephant_DriverRunToIdle`, :5269) names `elephant_mo_relaxed_runtoidle_f`, which the 1.2 `.cal` and
    /// pak lack; it crossfades directly instead of playing another mount's clip.
    fn run_to_idle(self) -> Option<&'static str> {
        match self {
            MountKind::Lion | MountKind::Bear => Some("lion_mo_relaxed_runtoidle_f"),
            MountKind::Wildboar => Some("wildboar_mo_relaxed_runtoidle_f"),
            MountKind::Horse | MountKind::Pangolin => Some("horse_mo_relaxed_runtoidle_f"),
            MountKind::Elephant => None,
        }
    }
}

/// The get-on reason of an attach: `SCUnitAttached` 0x076 carries one `reason` byte (reader `FUN_397c60c0`). The
/// 1.2 client passes it to the get-on state: the attach model maps 2/3/4 else 1 (`FUN_390eb520:187315-187327`) and
/// calls `FUN_390bb4d0`, the thunk that calls `FUN_39153b60` (`x2game.dll.c:154663`). AAEmu echoes the
/// `CSMountMate` reason in the reply (`mates.rs:340-344`) and sends the `bond_kind` for slave seats
/// (`slaves.rs:1383-1387`); the original server's value is unverified.
fn get_on_reason(reason: u8) -> u8 {
    if (1..=4).contains(&reason) { reason } else { 1 }
}

/// The get-on reason a remote attach plays: a rider that was already visible on foot gets the reply's reason;
/// a rider that streams in already seated starts in the riding loops (`None`).
fn fresh_get_on_reason(fresh: bool, reason: u8) -> Option<u8> {
    fresh.then(|| get_on_reason(reason))
}

/// The get-on clip of a reason: `Horse_Enter_l` :1517 (seat 1), `_r` :1521 (seat 2), `_b` :1509 (seat 3),
/// `_f` :1513 (seat 4); `FUN_39153b60` gives the other seats seat 1, so every other reason takes the left clip.
/// Bear (:1337-1351), lion (:1645-1655) and wildboar (:1889-1899) have their own `Enter` states; the elephant
/// (:1417-1427) and the pangolin (:1733-1745) use the horse's.
fn get_on_clip(kind: MountKind, reason: u8) -> String {
    let prefix = match kind {
        MountKind::Bear => "bear",
        MountKind::Lion | MountKind::Wildboar => "lion",
        _ => "horse",
    };
    let side = match reason {
        2 => "r",
        3 => "b",
        4 => "f",
        _ => "l",
    };
    format!("fist_ac_{prefix}_{side}_geton")
}

/// The get-off clips (`jump_horse_start` :11742 -> `_loop` :11736 -> `_end` :11733); every mount kind's
/// `jump_*` states use the horse's left clips (`jump_bear_*` :11637, `jump_elephant_*` :11653,
/// `jump_pangolin_*` :12636, `jump_wildboar_*` :13055; horse :17356 and lion :17357 both end at their `End`).
const GET_OFF_START: &str = "fist_ac_horse_l_getoff_start";
const GET_OFF_LOOP: &str = "fist_ac_horse_l_getoff_loop";
const GET_OFF_END: &str = "fist_ac_horse_l_getoff_end";

/// The `transition_time` of the idle-to-run and run-to-idle states (:1474, :1488): the crossfade into the one-shot,
/// not its length (the clips are 0.6-2.7 s long).
const GAIT_BLEND: f32 = 0.2;

/// The bone of a mount a rider at `point` sits on (`model_attach_point_strings`: `$driver` is `bone_spine_driver`).
fn seat_bone(point: u8) -> &'static str {
    if point <= 1 { "bone_spine_driver" } else { "bone_spine_passenger" }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gait {
    Idle,
    Walk,
    Run,
}

/// The rider's gait for the mount's ground speed (m/s). The bands are the graph's `PseudoSpeed` ones:
/// `DriverIdle` 0..0.1 (:1464), `DriverWalk` 0.15..0.6 (:1490), `DriverRun` 0.7..2 (:1479). PseudoSpeed is not
/// m/s: 1.2 sets it to 1 at an actor's full run (the Dash controller, `docs/research/re-skills.md`), so the
/// speed is divided by the mount's `MountSpeed::run`. The bands leave 0.1..0.15 and 0.6..0.7 unselected; they
/// keep the lower state. The scale is unverified against the real client.
fn gait(speed: f32, run_speed: f32, has_run: bool) -> Gait {
    let pseudo = speed / run_speed.max(0.1);
    if pseudo < 0.15 {
        Gait::Idle
    } else if pseudo < 0.7 || !has_run {
        Gait::Walk
    } else {
        Gait::Run
    }
}

/// A rider clip of the 1.2 graph.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RiderClip {
    GetOn,
    Idle,
    Walk,
    Run,
    IdleToRun,
    RunToIdle,
    GetOffStart,
    GetOffLoop,
    GetOffEnd,
}

impl RiderClip {
    fn looping(self) -> bool {
        matches!(self, RiderClip::Idle | RiderClip::Walk | RiderClip::Run)
    }
}

/// What the state machine wants this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RideStep {
    None,
    Play(RiderClip),
    /// The get-off clips are over: the unit walks again.
    OnFoot,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RidePhase {
    /// The get-on one-shot of the mount reason, then [`RidePhase::Riding`].
    Mounting,
    Riding,
    Dismounting,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GetOff {
    Start,
    Loop,
    End,
    Done,
}

/// The lengths of the one-shots (s), the clips' own: the get-on and get-off chains, and the gait transition one-shots
/// whose 0.2 s graph value is only the blend ([`GAIT_BLEND`]).
#[derive(Clone, Copy, Debug, Default)]
struct RiderDurations {
    get_on: Option<f32>,
    idle_to_run: Option<f32>,
    run_to_idle: Option<f32>,
    get_off_start: Option<f32>,
    get_off_loop: Option<f32>,
    get_off_end: Option<f32>,
}

/// The rider's animation state, split from the clips so it can be stepped in tests: the get-on one-shot, the
/// graph's gait loop with its idle-to-run / run-to-idle one-shots, and the get-off start/loop/end chain.
#[derive(Clone, Copy, Debug)]
struct RiderAnim {
    phase: RidePhase,
    gait: Option<Gait>,
    /// The idle-to-run / run-to-idle one-shot is playing; its end chooses the gait again.
    transition: bool,
    get_off: GetOff,
    left: f32,
    entered: bool,
    durations: RiderDurations,
}

impl RiderAnim {
    /// `phase` is [`RidePhase::Mounting`] for a fresh attach (the get-on one-shot) and [`RidePhase::Riding`] for a
    /// rider that was already seated (it streamed in riding, or the player entity was built again).
    fn new(durations: RiderDurations, phase: RidePhase) -> RiderAnim {
        RiderAnim {
            phase,
            gait: None,
            transition: false,
            get_off: GetOff::Start,
            left: 0.0,
            entered: false,
            durations,
        }
    }

    /// Starts the get-off chain; `false` when the mount has no get-off clips.
    fn begin_dismount(&mut self) -> bool {
        if self.durations.get_off_start.is_none() {
            return false;
        }
        self.phase = RidePhase::Dismounting;
        self.gait = None;
        self.transition = false;
        self.get_off = GetOff::Start;
        self.left = 0.0;
        self.entered = false;
        true
    }

    fn advance(&mut self, dt: f32, speed: f32, run_speed: f32, has_run: bool) -> RideStep {
        match self.phase {
            RidePhase::Mounting => {
                if !self.entered {
                    self.entered = true;
                    if let Some(length) = self.durations.get_on {
                        self.left = length;
                        return RideStep::Play(RiderClip::GetOn);
                    }
                    self.phase = RidePhase::Riding;
                } else {
                    self.left -= dt;
                    if self.left > 0.0 {
                        return RideStep::None;
                    }
                    self.phase = RidePhase::Riding;
                }
                self.riding(dt, speed, run_speed, has_run)
            }
            RidePhase::Riding => self.riding(dt, speed, run_speed, has_run),
            RidePhase::Dismounting => self.dismounting(dt),
        }
    }

    fn riding(&mut self, dt: f32, speed: f32, run_speed: f32, has_run: bool) -> RideStep {
        if self.transition {
            self.left -= dt;
            if self.left > 0.0 {
                return RideStep::None;
            }
            // the one-shot played to its clip's end (`leave_at_stop`): the gait is chosen again from this frame's
            // PseudoSpeed instead of the one the one-shot started with
            self.transition = false;
            self.gait = None;
        }
        let wanted = gait(speed, run_speed, has_run);
        if self.gait == Some(wanted) {
            return RideStep::None;
        }
        let previous = self.gait.replace(wanted);
        match (previous, wanted) {
            (Some(Gait::Idle), Gait::Run) if self.durations.idle_to_run.is_some() => {
                self.transition = true;
                self.left = self.durations.idle_to_run.unwrap_or(0.0);
                RideStep::Play(RiderClip::IdleToRun)
            }
            (Some(Gait::Run), Gait::Idle) if self.durations.run_to_idle.is_some() => {
                self.transition = true;
                self.left = self.durations.run_to_idle.unwrap_or(0.0);
                RideStep::Play(RiderClip::RunToIdle)
            }
            _ => RideStep::Play(match wanted {
                Gait::Idle => RiderClip::Idle,
                Gait::Walk => RiderClip::Walk,
                Gait::Run => RiderClip::Run,
            }),
        }
    }

    fn dismounting(&mut self, dt: f32) -> RideStep {
        if !self.entered {
            self.entered = true;
            if let Some(length) = self.durations.get_off_start {
                self.left = length;
                return RideStep::Play(RiderClip::GetOffStart);
            }
        } else {
            self.left -= dt;
            if self.left > 0.0 {
                return RideStep::None;
            }
        }
        loop {
            match self.get_off {
                GetOff::Start => {
                    self.get_off = GetOff::Loop;
                    if let Some(length) = self.durations.get_off_loop {
                        self.left = length;
                        return RideStep::Play(RiderClip::GetOffLoop);
                    }
                }
                GetOff::Loop => {
                    self.get_off = GetOff::End;
                    if let Some(length) = self.durations.get_off_end {
                        self.left = length;
                        return RideStep::Play(RiderClip::GetOffEnd);
                    }
                }
                GetOff::End | GetOff::Done => {
                    self.get_off = GetOff::Done;
                    return RideStep::OnFoot;
                }
            }
        }
    }
}

/// The clips a rider can play.
struct RiderClips {
    get_on: Option<BoundClip>,
    idle: BoundClip,
    walk: Option<BoundClip>,
    run: Option<BoundClip>,
    idle_to_run: Option<BoundClip>,
    run_to_idle: Option<BoundClip>,
    get_off_start: Option<BoundClip>,
    get_off_loop: Option<BoundClip>,
    get_off_end: Option<BoundClip>,
}

impl RiderClips {
    fn get(&self, clip: RiderClip) -> Option<&BoundClip> {
        match clip {
            RiderClip::GetOn => self.get_on.as_ref(),
            RiderClip::Idle => Some(&self.idle),
            RiderClip::Walk => self
                .walk
                .as_ref()
                .or(self.run.as_ref())
                .or(Some(&self.idle)),
            RiderClip::Run => self.run.as_ref(),
            RiderClip::IdleToRun => self.idle_to_run.as_ref(),
            RiderClip::RunToIdle => self.run_to_idle.as_ref(),
            RiderClip::GetOffStart => self.get_off_start.as_ref(),
            RiderClip::GetOffLoop => self.get_off_loop.as_ref(),
            RiderClip::GetOffEnd => self.get_off_end.as_ref(),
        }
    }

    fn durations(&self) -> RiderDurations {
        RiderDurations {
            get_on: self.get_on.as_ref().map(|c| c.clip.duration),
            idle_to_run: self.idle_to_run.as_ref().map(|c| c.clip.duration),
            run_to_idle: self.run_to_idle.as_ref().map(|c| c.clip.duration),
            get_off_start: self.get_off_start.as_ref().map(|c| c.clip.duration),
            get_off_loop: self.get_off_loop.as_ref().map(|c| c.clip.duration),
            get_off_end: self.get_off_end.as_ref().map(|c| c.clip.duration),
        }
    }
}

/// The forward example of a locomotion group: the `_f` clip nearest the forward direction (0, 1), the direction
/// suffix `ingame::anim` builds its clip names from. The rider groups also list turn, uphill and downhill variants on
/// the forward axis, so the nearest example alone can be the wrong one; without an `_f` example the nearest is taken.
fn forward_example(lmg: &aa_anim::Lmg) -> Option<&aa_anim::lmg::Example> {
    let forward = Vec2::new(0.0, 1.0);
    let distance = |e: &aa_anim::lmg::Example| e.direction.normalize_or_zero().distance(forward);
    let is_forward = |e: &aa_anim::lmg::Example| e.animation.rsplit('_').next() == Some("f");
    lmg.examples
        .iter()
        .filter(|e| is_forward(e))
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .or_else(|| {
            lmg.examples
                .iter()
                .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        })
}

/// Binds a rider clip by its `.cal` name, or by the locomotion group a graph animation names
/// (`_FIST_MO_RIDER_HORSE_RUN`): its forward example is the forward clip.
fn bind_rider_clip(rig: &Rig, cal: &Cal, clips: &ClipLibrary, name: &str) -> Option<BoundClip> {
    bind_clip(rig, cal, clips, name).or_else(|| {
        let group = cal.locomotion.get(&name.to_ascii_lowercase())?;
        let lmg = aa_anim::Lmg::load(&group.path).ok()?;
        let animation = forward_example(&lmg)?.animation.clone();
        bind_clip(rig, cal, clips, &animation)
    })
}

/// A unit sitting on a mount (a remote unit, or the local player).
#[derive(Component)]
pub struct Rider {
    /// The rider unit's object id (the log names it; the seat and clips are the same for every rider).
    unit: u32,
    mount: u32,
    point: u8,
    clips: RiderClips,
    anim: RiderAnim,
    /// The mount's base run speed (m/s): PseudoSpeed 1 in the graph, see [`gait`].
    run_speed: f32,
    /// The locomotion the unit had on foot, given back when it gets off.
    on_foot: Option<Locomotion>,
}

impl Rider {
    /// Binds the rider clips of `character` for a seat on `mount`, a unit of model `mount_model`; `reason` is the
    /// get-on reason (1..4; the local request's `FUN_3909ae90` or the attach reply's) of a fresh attach, `None` for a
    /// rider that was already seated (it streamed in riding, or the player entity was built again), which starts in
    /// the riding loops.
    fn bind(
        db: &Db,
        clips: &ClipLibrary,
        character: &Character,
        animator: &Animator,
        mount: Attachment,
        mount_model: u32,
        reason: Option<u8>,
        unit: u32,
    ) -> Option<Rider> {
        let pose = db.mount_pose(mount_model);
        let kind = MountKind::of(pose);
        let states = rider_states(pose);
        let run_speed = db.mount_speed(mount_model).run;
        let cal = Cal::shared(&character.assets.cal).ok()?;
        let bind = |name: &str| bind_rider_clip(&animator.rig, &cal, clips, name);
        let idle = bind(kind.idle()).or_else(|| bind("fist_ba_relaxed_idle"))?;
        let run = bind(kind.run()).or_else(|| bind(kind.run_fallback()));
        let walk = bind(kind.walk()).or_else(|| run.clone());
        let idle_to_run = if states {
            bind(kind.idle_to_run()).or_else(|| kind.idle_to_run_fallback().and_then(bind))
        } else {
            None
        };
        let run_to_idle = if states {
            kind.run_to_idle().and_then(bind)
        } else {
            None
        };
        let get_on = if states {
            let reason = get_on_reason(reason.unwrap_or(1));
            bind(&get_on_clip(kind, reason)).or_else(|| bind(&get_on_clip(kind, 1)))
        } else {
            None
        };
        let clips = RiderClips {
            get_on,
            idle,
            walk,
            run,
            idle_to_run,
            run_to_idle,
            get_off_start: if states { bind(GET_OFF_START) } else { None },
            get_off_loop: if states { bind(GET_OFF_LOOP) } else { None },
            get_off_end: if states { bind(GET_OFF_END) } else { None },
        };
        let anim = RiderAnim::new(
            clips.durations(),
            if reason.is_some() {
                RidePhase::Mounting
            } else {
                RidePhase::Riding
            },
        );
        Some(Rider {
            unit,
            mount: mount.obj_id,
            point: mount.point,
            clips,
            anim,
            run_speed,
            on_foot: None,
        })
    }

    pub(crate) fn dismounting(&self) -> bool {
        self.anim.phase == RidePhase::Dismounting
    }

    /// Steps the rider animation: `speed` the mount's ground speed (m/s), `tempo` its animator's speed.
    /// `true` once the get-off clips are over and the unit should walk again.
    fn step(&mut self, animator: &mut Animator, dt: f32, speed: f32, tempo: f32) -> bool {
        // A frame-held animator's clock stops (`Animator::update`), and `Animator::play` rejects new starts, so the
        // rider's one-shot timers must stop with it: otherwise the get-on finishes behind the hold and the get-off
        // chain can end (and take the rider off) before its clips ever play.
        if !animator.starts_enabled() {
            return false;
        }
        let step = self
            .anim
            .advance(dt, speed, self.run_speed, self.clips.run.is_some());
        match step {
            RideStep::None => {}
            RideStep::Play(clip) => {
                if let Some(bound) = self.clips.get(clip).cloned() {
                    let file = super::anim::clip_file(&bound);
                    debug!(
                        "rider {} phase {:?} clip {:?} file {}",
                        self.unit,
                        self.anim.phase,
                        clip,
                        file.as_deref().unwrap_or("-")
                    );
                    animator.play(bound, clip.looping(), GAIT_BLEND);
                    animator.random_idle = None;
                }
            }
            RideStep::OnFoot => {
                animator.speed = 1.0;
                return true;
            }
        }
        let looping_gait = self.anim.phase == RidePhase::Riding && !self.anim.transition;
        animator.speed = if looping_gait && matches!(self.anim.gait, Some(Gait::Walk | Gait::Run)) {
            tempo
        } else {
            1.0
        };
        false
    }
}

/// Seats a rider; its on-foot locomotion is kept for later.
fn seat(world: &mut World, entity: Entity, mut rider: Rider) {
    rider.on_foot = world.entity_mut(entity).take::<Locomotion>();
    world.entity_mut(entity).insert(rider);
}

/// Lets a rider walk again.
fn unseat(world: &mut World, entity: Entity) {
    let Some(rider) = world.entity_mut(entity).take::<Rider>() else { return };
    if let Some(mut on_foot) = rider.on_foot {
        on_foot.invalidate();
        world.entity_mut(entity).insert(on_foot);
    }
    if let Some(mut motion) = world.entity_mut(entity).get_mut::<Motion>() {
        *motion = Motion::default();
    }
}

/// Where the seat bone of a mount is now (Bevy coordinates): the bone's place on the mount's last drawn pose, carried to where
/// the mount stands in this frame (`now`), so a rider does not trail a moving mount by a frame.
fn seat_world(mount: Entity, now: &Transform, animator: &Animator, point: u8, globals: &Query<&GlobalTransform>) -> Vec3 {
    let bone = animator.rig.skeleton.bones.iter().position(|b| b.name == seat_bone(point)).and_then(|i| animator.bones.get(i));
    match (globals.get(mount), bone.and_then(|b| globals.get(*b).ok())) {
        (Ok(root), Some(bone)) => now.transform_point(root.affine().inverse().transform_point3(bone.translation())),
        _ => now.translation,
    }
}

/// The attachment each remote unit with an entity was last seen with. A unit seen on foot whose attach then arrives
/// plays the get-on one-shot; a unit first seen already attached, or a rider that leaves the view and returns
/// mounted, starts in the riding loops. The real client keeps the per-entity attach event; this is a conservative
/// rule, not proven by the 1.2 decompile.
#[derive(Default)]
pub(crate) struct Seen(std::collections::HashMap<u32, Option<(u32, u8)>>);

impl Seen {
    /// `true` when this client last saw `obj_id` on foot: the attach that follows is a get-on.
    fn saw_on_foot(&self, obj_id: u32) -> bool {
        matches!(self.0.get(&obj_id), Some(None))
    }

    /// Records the attachment this frame. A rider first seen attached is never fresh, and a rider seen on foot keeps
    /// that state while its attach waits for the animator or the mount's data: the wait neither turns a seated rider
    /// into a fresh get-on nor loses the get-on of a unit that was really seen on foot.
    /// Drops the units that have no entity any more (out of view, despawned, a world switch or a logout): one that
    /// comes back is first seen again, so a rider returning already mounted does not replay its get-on, and a reused
    /// object id starts with no history.
    fn forget_absent(&mut self, present: &std::collections::HashSet<u32>) {
        self.0.retain(|id, _| present.contains(id));
    }

    fn observe(&mut self, obj_id: u32, attached: Option<(u32, u8)>, seated: bool) {
        if attached.is_some() && !seated && matches!(self.0.get(&obj_id), Some(None)) {
            return;
        }
        self.0.insert(obj_id, attached);
    }
}

/// `true` when the attach of a unit is a fresh get-on: it has no rider yet and this client last saw it on foot.
fn fresh_attach(on_foot: bool, attached: Option<(u32, u8)>, seen: &Seen, obj_id: u32) -> bool {
    on_foot && attached.is_some() && seen.saw_on_foot(obj_id)
}

/// What a unit's attachment asks of the seating systems this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SeatAction {
    None,
    /// Attach with no rider: seat it.
    Mount,
    /// A rider on another seat, or attached again while getting off: unseat it at once and seat it again.
    Remount,
    /// Detached and not already getting off: play the get-off clips.
    Dismount,
}

fn seat_action(attached: Option<(u32, u8)>, rider: Option<(u32, u8, bool)>) -> SeatAction {
    match (attached, rider) {
        (None, None) => SeatAction::None,
        (None, Some((_, _, true))) => SeatAction::None,
        (None, Some(_)) => SeatAction::Dismount,
        (Some(_), None) => SeatAction::Mount,
        (Some(a), Some((mount, point, _))) if (mount, point) != a => SeatAction::Remount,
        (Some(_), Some((_, _, true))) => SeatAction::Remount,
        (Some(_), Some(_)) => SeatAction::None,
    }
}

/// Seats the units the server attached to a mount and lets the ones that got off walk again.
#[allow(clippy::type_complexity)]
pub fn start_stop_riding(
    mut commands: Commands,
    net: Res<Units>,
    db: Option<Res<Db>>,
    clips: Res<ClipLibrary>,
    units: Query<(Entity, &RemoteUnit, Option<&Rider>, Option<&Character>, Option<&Animator>)>,
    mut seen: Local<Seen>,
) {
    let mut present = std::collections::HashSet::new();
    for (entity, unit, rider, character, animator) in &units {
        let attached = net.by_id.get(&unit.obj_id).and_then(|u| u.attached_to);
        let seat_of = attached.map(|a| (a.obj_id, a.point));
        let action = seat_action(seat_of, rider.map(|r| (r.mount, r.point, r.dismounting())));
        let fresh = fresh_attach(rider.is_none(), seat_of, &seen, unit.obj_id);
        let mut seated = false;
        match action {
            SeatAction::Mount | SeatAction::Remount => {
                // wait for the animator (a unit far away has none) and the mount's data
                let mount = attached.and_then(|a| net.by_id.get(&a.obj_id));
                if let (Some(a), Some(db), Some(character), Some(animator), Some(mount)) =
                    (attached, db.as_deref(), character, animator, mount)
                {
                    // a fresh remote attach plays the get-on of the reply's reason, which the 1.2 client maps and
                    // passes to FUN_39153b60 (FUN_390eb520 -> FUN_390bb4d0); AAEmu echoes the request's reason (the
                    // original server's value is unverified)
                    let reason = fresh_get_on_reason(fresh, a.reason);
                    if let Some(rider) = Rider::bind(
                        db,
                        &clips,
                        character,
                        animator,
                        a,
                        mount.model_id,
                        reason,
                        unit.obj_id,
                    ) {
                        debug!(
                            "unit {} rides {} (model {}) at point {}",
                            unit.obj_id, a.obj_id, mount.model_id, a.point
                        );
                        if action == SeatAction::Remount {
                            commands.queue(move |world: &mut World| {
                                unseat(world, entity);
                                seat(world, entity, rider);
                            });
                        } else {
                            commands.queue(move |world: &mut World| seat(world, entity, rider));
                        }
                        seated = true;
                    }
                }
            }
            SeatAction::Dismount => {
                commands.queue(move |world: &mut World| {
                    // it stands where the mount was: the movement packets of the unit itself carry on from here
                    if let Some(mut unit) = world.entity_mut(entity).get_mut::<RemoteUnit>() {
                        let (pos, facing) = (unit.pos, unit.facing);
                        unit.reset_track(pos, facing);
                    }
                    if !start_dismount(world, entity) {
                        unseat(world, entity);
                    }
                });
            }
            SeatAction::None => {}
        }
        seen.observe(unit.obj_id, seat_of, seated);
        present.insert(unit.obj_id);
    }
    seen.forget_absent(&present);
}

/// Puts the riders on their mounts' seats and plays the rider clips for the gait of the mount.
pub fn pose_riders(
    time: Res<Time>,
    net: Res<Units>,
    views: Res<Views>,
    mut riders: Query<(&mut RemoteUnit, &mut Transform, &mut Rider, &mut Animator)>,
    mounts: Query<(&RemoteUnit, &Transform, &Animator, &Motion), Without<Rider>>,
    globals: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    for (mut unit, mut transform, mut rider, mut animator) in &mut riders {
        // a rider getting off stays where the server put it, while step_dismounts plays the get-off clips
        if rider.dismounting() {
            continue;
        }
        let mounted = views.unit_entity(rider.mount).and_then(|e| mounts.get(e).ok().map(|m| (e, m)));
        let (pos, facing, speed, tempo) = match mounted {
            Some((e, (mount, now, mount_animator, motion))) => {
                let world = seat_world(e, now, mount_animator, rider.point, &globals);
                (bevy_to_cry(world), mount.facing, motion.speed, mount_animator.speed)
            }
            None => {
                // the mount left the view: stay with where the server says it is, or where the rider stands if the
                // mount is gone too; its one-shots still finish (the server detaches it next)
                let Some(m) = net.by_id.get(&rider.mount) else {
                    rider.step(&mut animator, dt, 0.0, 1.0);
                    continue;
                };
                (Vec3::new(m.pos.x, m.pos.y, m.pos.z), m.yaw(), 0.0, 1.0)
            }
        };
        unit.pos = pos;
        unit.facing = facing;
        *transform = placement(pos, facing, unit.scale());
        rider.step(&mut animator, dt, speed, tempo);
    }
}

/// The reason of the local player's last `CSMountMate` (`FUN_3909ae90`, 1..4): the get-on state of the mount graph
/// takes it (`FUN_39153b60` picks the seat); it is written when `hud/pets` sends the command and read when the server
/// attaches the player.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountReason(pub u8);

impl MountReason {
    /// The pending reason for an attach at `point` of a mount of `pose`: only the driver's seat of a mount that has
    /// the graph's rider states is the mount that was asked for. A passenger seat, a boat or a car drops it, so a
    /// reason left by a refused request is not applied to another unit.
    fn pending(&mut self, point: u8, pose: u32) -> Option<u8> {
        let reason = (1..=4).contains(&self.0).then_some(self.0);
        if reason.is_none() || point != 1 || !rider_states(pose) {
            self.0 = 0;
            return None;
        }
        reason
    }

    /// The bind that used the pending reason succeeded.
    fn used(&mut self) {
        self.0 = 0;
    }
}

/// Starts the get-off clips of a rider; `false` when it has none (it then walks at once).
fn start_dismount(world: &mut World, entity: Entity) -> bool {
    let Some(mut rider) = world.get_mut::<Rider>(entity) else {
        return false;
    };
    rider.anim.begin_dismount()
}

/// Plays the get-off start/loop/end clips of the riders that detached and lets them walk once the clips are over.
pub fn step_dismounts(
    mut commands: Commands,
    time: Res<Time>,
    mut riders: Query<(Entity, &mut Rider, &mut Animator)>,
) {
    let dt = time.delta_secs();
    for (entity, mut rider, mut animator) in &mut riders {
        if rider.dismounting() && rider.step(&mut animator, dt, 0.0, 1.0) {
            commands.queue(move |world: &mut World| unseat(world, entity));
        }
    }
}

/// The mount the local player sits on (`None` on foot), for the movement, the camera and the HUD.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct LocalMount(pub Option<LocalSeat>);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalSeat {
    /// The mount's object id and the attach point.
    pub mount: u32,
    pub point: u8,
    /// On the driver's seat (point 1, which the server gives only the mate's owner): the player's keys move the mount.
    pub drives: bool,
    /// The attached unit is a slave (a cart or other land vehicle), not a mate: `cart::drive_cart` owns it.
    pub cart: bool,
    pub speed: MountSpeed,
    /// Height of the seat over the mount's feet (m): the camera's pivot rises by it.
    pub lift: f32,
}

impl LocalMount {
    /// The mount the player drives: its object id and speed.
    pub fn driving(&self) -> Option<(u32, MountSpeed)> {
        self.0
            .filter(|s| s.drives && !s.cart)
            .map(|s| (s.mount, s.speed))
    }

    /// The slave cart the player drives from attach point 1.
    pub fn cart_driver(&self) -> Option<u32> {
        self.0.filter(|s| s.drives && s.cart).map(|s| s.mount)
    }

    /// On a seat it does not steer (a passenger).
    pub fn passenger(&self) -> bool {
        self.0.is_some_and(|s| !s.drives)
    }

    /// How far the camera's pivot rises over the feet: a rider's origin is on the seat, and its head is lower over it than a
    /// standing character's over its feet by about the hips' height ([`SEATED_DROP`]).
    pub fn lift(&self) -> f32 {
        self.0.map_or(0.0, |s| (s.lift - SEATED_DROP).max(0.0))
    }
}

/// How much lower a seated rider's head is over the seat than a standing character's over its feet (m).
const SEATED_DROP: f32 = 0.85;

/// Marks the unit of the mate the local player drives: [`drive_mount`] places it, not its movement packets.
#[derive(Component)]
pub struct Driven;

/// Follows the local player's attachment (`SCUnitAttached` / `SCUnitDetached`): seats it on the mount and puts its mover where
/// the mount is; when it gets off it walks on from there (the server puts it where the mount stood too).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn track_local_mount(
    mut commands: Commands,
    player: Res<Player>,
    net: Res<Units>,
    db: Option<Res<Db>>,
    clips: Res<ClipLibrary>,
    views: Res<Views>,
    (mut seat_state, mut send): (ResMut<LocalMount>, ResMut<SendState>),
    mut mount_reason: ResMut<MountReason>,
    mut local: Query<(Entity, &mut Mover, Option<&Rider>, Option<&Character>, Option<&Animator>), With<LocalPlayer>>,
    mut cameras: Query<&mut OrbitCamera>,
    mut units: Query<(&mut RemoteUnit, Has<Driven>)>,
) {
    // a mount that left the view is let go of (the server detaches the riders before it despawns a mate)
    let attached = player.spawn.as_ref().and_then(|s| s.unit.attached_to).filter(|a| net.by_id.contains_key(&a.obj_id));
    let Ok((entity, mut mover, rider, character, animator)) = local.single_mut() else { return };
    let old = seat_state.0;
    if old.map(|s| (s.mount, s.point)) != attached.map(|a| (a.obj_id, a.point)) {
        // the mate the player drove goes back to its movement packets, from where it stands
        if let Some(e) = old.filter(|s| s.drives).and_then(|s| views.unit_entity(s.mount)) {
            commands.entity(e).remove::<Driven>();
            if let Ok((mut unit, _)) = units.get_mut(e) {
                let (pos, facing) = (unit.pos, unit.facing);
                unit.reset_track(pos, facing);
            }
        }
        seat_state.0 = attached.map(|a| {
            let (model, cart) = net.by_id.get(&a.obj_id).map_or((0, false), |u| {
                (u.model_id, matches!(u.kind, UnitKind::Slave { .. }))
            });
            let speed = db.as_deref().map_or_else(MountSpeed::default, |db| db.mount_speed(model));
            LocalSeat { mount: a.obj_id, point: a.point, drives: a.point == 1, cart, speed, lift: 0.0 }
        });
        match attached {
            Some(a) => {
                // on the mount: where it is drawn (else where the server has it), facing its way
                let shown = views.unit_entity(a.obj_id).and_then(|e| units.get(e).ok()).map(|(u, _)| (u.pos, u.facing));
                let known = net.by_id.get(&a.obj_id).map(|u| (Vec3::new(u.pos.x, u.pos.y, u.pos.z), u.yaw()));
                if let Some((pos, facing)) = shown.or(known) {
                    mover.pos = pos;
                    mover.facing = facing;
                    if let Ok(mut orbit) = cameras.single_mut() {
                        orbit.yaw = facing;
                    }
                }
                info!("riding {} at point {}: {:?}", a.obj_id, a.point, seat_state.0.map(|s| s.speed));
            }
            None => info!("got off the mount at ({:.1}, {:.1}, {:.1})", mover.pos.x, mover.pos.y, mover.pos.z),
        }
        mover.velocity = Vec3::ZERO;
        mover.vz = 0.0;
        mover.swim = None;
        mover.grounded = true;
        send.send_now();
    }
    // the driven mate's unit (built again when it comes back into view) follows the mover
    if let Some(e) = seat_state.0.filter(|s| s.drives).and_then(|s| views.unit_entity(s.mount))
        && units.get(e).is_ok_and(|(_, driven)| !driven)
    {
        commands.entity(e).insert(Driven);
    }
    // the rider's clips (also for a player entity built again while riding)
    let action = seat_action(
        attached.map(|a| (a.obj_id, a.point)),
        rider.map(|r| (r.mount, r.point, r.dismounting())),
    );
    match action {
        SeatAction::Mount | SeatAction::Remount => {
            let Some(a) = attached else { return };
            let (Some(db), Some(character), Some(animator)) = (db.as_deref(), character, animator) else { return };
            let model = net.by_id.get(&a.obj_id).map_or(0, |u| u.model_id);
            // the local player keeps using the reason computed when it sent CSMountMate: AAEmu echoes it in the
            // reply (mates.rs:340-344; the original server is unverified), but a player entity built again while
            // riding is attached too, and only the pending request tells a fresh get-on from that
            let reason = mount_reason.pending(a.point, db.mount_pose(model));
            let Some(rider) = Rider::bind(db, &clips, character, animator, a, model, reason,
                                           player.spawn.as_ref().map_or(0, |s| s.obj_id)) else { return };
            if reason.is_some() {
                mount_reason.used();
            }
            if action == SeatAction::Remount {
                commands.queue(move |world: &mut World| {
                    unseat(world, entity);
                    seat(world, entity, rider);
                });
            } else {
                commands.queue(move |world: &mut World| seat(world, entity, rider));
            }
        }
        SeatAction::Dismount => {
            mount_reason.0 = 0;
            commands.queue(move |world: &mut World| {
                if !start_dismount(world, entity) {
                    unseat(world, entity);
                }
            });
        }
        SeatAction::None => {}
    }
}

/// The mate the player drives stands where the player's mover is and plays its gait for the mover's speed; a passenger's mover
/// goes with the mount.
#[allow(clippy::type_complexity)]
pub fn drive_mount(
    seat: Res<LocalMount>,
    views: Res<Views>,
    net: Res<Units>,
    mut player: Query<&mut Mover, With<LocalPlayer>>,
    mut driven: Query<(&mut RemoteUnit, &mut Transform, Option<&mut Motion>), (With<Driven>, Without<LocalPlayer>)>,
    mounts: Query<&RemoteUnit, Without<Driven>>,
) {
    let (Some(s), Ok(mut mover)) = (seat.0, player.single_mut()) else { return };
    // a cart's driver is moved by `cart::drive_cart`; a cart passenger still follows the cart here
    if s.cart && s.drives {
        return;
    }
    let entity = views.unit_entity(s.mount);
    if !s.drives {
        let shown = entity.and_then(|e| mounts.get(e).ok()).map(|u| (u.pos, u.facing));
        let known = net.by_id.get(&s.mount).map(|u| (Vec3::new(u.pos.x, u.pos.y, u.pos.z), u.yaw()));
        if let Some((pos, facing)) = shown.or(known) {
            mover.pos = pos;
            mover.facing = facing;
        }
        return;
    }
    let Some((mut unit, mut transform, motion)) = entity.and_then(|e| driven.get_mut(e).ok()) else { return };
    unit.pos = mover.pos;
    unit.facing = mover.facing;
    *transform = placement(unit.pos, unit.facing, unit.scale());
    if let Some(mut motion) = motion {
        let speed = mover.velocity.truncate().length();
        let moving = speed > 0.3;
        *motion = Motion { speed: if moving { speed } else { 0.0 }, local_dir: if moving { Vec2::Y } else { Vec2::ZERO }, airborne: false };
    }
}

/// Puts the local player on its mount's seat and plays the rider clips for the mount's gait.
#[allow(clippy::type_complexity)]
pub fn pose_local_rider(
    time: Res<Time>,
    mut seat: ResMut<LocalMount>,
    views: Res<Views>,
    mut riders: Query<(&Mover, &mut Transform, &mut Rider, &mut Animator), With<LocalPlayer>>,
    mounts: Query<(&Transform, &Animator, &Motion), (With<RemoteUnit>, Without<LocalPlayer>)>,
    globals: Query<&GlobalTransform>,
) {
    let (Some(s), Ok((mover, mut transform, mut rider, mut animator))) = (seat.0, riders.single_mut()) else { return };
    if rider.dismounting() {
        return;
    }
    let mounted = views.unit_entity(s.mount).and_then(|e| mounts.get(e).ok().map(|m| (e, m)));
    let (pos, speed, tempo) = match mounted {
        Some((e, (now, mount_animator, motion))) => (bevy_to_cry(seat_world(e, now, mount_animator, s.point, &globals)), motion.speed, mount_animator.speed),
        None => (mover.pos, 0.0, 1.0),
    };
    *transform = placement(pos, mover.facing, 1.0);
    rider.step(&mut animator, time.delta_secs(), speed, tempo);
    let lift = (pos.z - mover.pos.z).max(0.0);
    if (lift - s.lift).abs() > 0.01 {
        debug!("seat of {} at ({:.2}, {:.2}, {:.2}), {lift:.2} m over the mount's feet at ({:.2}, {:.2}, {:.2})", s.mount, pos.x, pos.y, pos.z, mover.pos.x, mover.pos.y, mover.pos.z);
        seat.0 = Some(LocalSeat { lift, ..s });
    }
}

/// `AA_FAKE_MOUNT=1`, a check for the riders without a server: a horse (model 139) and a rider that looks like the local
/// player appear beside the player, the rider is attached to the horse, and the horse runs a circle at 5 m/s, reported ten
/// times a second like the walkthrough's circle. `AA_FAKE_MOUNT=still` parks the horse 4 m from the player instead (for a
/// close look at the seat).
pub fn fake_mount(time: Res<Time>, player: Res<Player>, mut units: ResMut<Units>, mut events: MessageWriter<NetEvent>, mut state: Local<Option<(f32, f32)>>) {
    const HORSE: u32 = 9_000_001;
    const RIDER: u32 = 9_000_002;
    const RADIUS: f32 = 8.0;
    const SPEED: f32 = 5.0;
    if std::env::var("AA_FAKE_MOUNT").is_err() {
        return;
    }
    let Some(spawn) = &player.spawn else { return };
    let now = time.elapsed_secs();
    let still = std::env::var("AA_FAKE_MOUNT").is_ok_and(|v| v == "still");
    let centre = Pos::new(spawn.unit.pos.x + if still { 4.0 } else { RADIUS + 3.0 }, spawn.unit.pos.y, spawn.unit.pos.z);
    let (started, last_sent) = *state.get_or_insert_with(|| {
        let mut horse = spawn.unit.clone();
        horse.obj_id = HORSE;
        horse.name = "Horse".into();
        horse.kind = UnitKind::Mate { tl_id: 1, template_id: 0, owner_id: 0 };
        horse.model_id = 139;
        horse.equipment = Equipment::default();
        horse.model_params = ModelParams::default();
        horse.pos = Pos::new(centre.x, centre.y, centre.z);
        let mut rider = spawn.unit.clone();
        rider.obj_id = RIDER;
        rider.name = "Rider".into();
        rider.pos = horse.pos;
        units.by_id.insert(HORSE, horse);
        units.by_id.insert(RIDER, rider);
        (now, 0.0)
    });
    // seat the rider once the horse has been announced for a moment
    if now - started > 4.0
        && let Some(rider) = units.by_id.get_mut(&RIDER)
    {
        rider.attached_to.get_or_insert(Attachment { obj_id: HORSE, point: 1, reason: 1 });
    }
    if now - last_sent < 0.1 || now - started < 2.0 || (still && last_sent > 0.0) {
        return;
    }
    let t = if still { 0.0 } else { now - started };
    let angle = t * SPEED / RADIUS;
    let pos = Pos::new(centre.x + RADIUS * angle.cos() - RADIUS, centre.y + RADIUS * angle.sin(), centre.z);
    let yaw = (SPEED * angle.sin()).atan2(SPEED * angle.cos());
    let mut movement = Movement::unit(pos, yaw, (t * 1000.0) as u32);
    if !still {
        movement.velocity = [(-yaw.sin() * SPEED * 1024.0) as i16, (yaw.cos() * SPEED * 1024.0) as i16, 0];
        movement.delta_movement = [0, 127, 0];
    }
    if let Some(horse) = units.by_id.get_mut(&HORSE) {
        horse.pos = pos;
    }
    events.write(NetEvent(Event::UnitMoved { obj_id: HORSE, movement }));
    *state = Some((started, now));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rider whose one-shots last 1 s (get-on), 0.6 / 0.45 s (the gait transition clips) and 0.67 / 0.5 / 0.83 s
    /// (get-off).
    fn durations() -> RiderDurations {
        RiderDurations {
            get_on: Some(1.0),
            idle_to_run: Some(0.6),
            run_to_idle: Some(0.45),
            get_off_start: Some(0.67),
            get_off_loop: Some(0.5),
            get_off_end: Some(0.83),
        }
    }

    fn mounted() -> RiderAnim {
        let mut anim = RiderAnim::new(durations(), RidePhase::Mounting);
        assert_eq!(
            anim.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::GetOn)
        );
        assert_eq!(
            anim.advance(1.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::Idle)
        );
        anim
    }

    fn riding() -> RiderAnim {
        RiderAnim::new(durations(), RidePhase::Riding)
    }

    #[test]
    fn the_mount_reason_picks_the_get_on_clip() {
        // player_full_body.xml Horse_Enter_l :1517, _r :1521, _b :1509, _f :1513; the other seats are seat 1
        assert_eq!(get_on_clip(MountKind::Horse, 1), "fist_ac_horse_l_geton");
        assert_eq!(get_on_clip(MountKind::Horse, 2), "fist_ac_horse_r_geton");
        assert_eq!(get_on_clip(MountKind::Horse, 3), "fist_ac_horse_b_geton");
        assert_eq!(get_on_clip(MountKind::Horse, 4), "fist_ac_horse_f_geton");
        assert_eq!(get_on_clip(MountKind::Horse, 0), "fist_ac_horse_l_geton");
        assert_eq!(get_on_clip(MountKind::Horse, 9), "fist_ac_horse_l_geton");
        assert_eq!(get_on_clip(MountKind::Lion, 2), "fist_ac_lion_r_geton");
        assert_eq!(get_on_clip(MountKind::Bear, 3), "fist_ac_bear_b_geton");
        assert_eq!(
            get_on_clip(MountKind::Wildboar, 4),
            "fist_ac_lion_f_geton",
            "wildboar uses the lion states"
        );
        assert_eq!(get_on_clip(MountKind::Elephant, 4), "fist_ac_horse_f_geton");
        assert_eq!(get_on_clip(MountKind::Pangolin, 3), "fist_ac_horse_b_geton");
    }

    #[test]
    fn an_attach_reason_outside_one_to_four_maps_to_one() {
        // the four sides of the 2026-10-28 capture are the reasons themselves
        for reason in 1..=4 {
            assert_eq!(get_on_reason(reason), reason);
        }
        // FUN_39153b60 gives the graph's other seats reason 1
        for reason in [0, 5, 9, 10, 255] {
            assert_eq!(get_on_reason(reason), 1, "reason {reason}");
        }
    }

    #[test]
    fn a_remote_rider_gets_on_with_the_attach_reply_reason() {
        assert_eq!(fresh_get_on_reason(true, 2), Some(2), "right");
        assert_eq!(fresh_get_on_reason(true, 3), Some(3), "behind");
        assert_eq!(fresh_get_on_reason(true, 4), Some(4), "front");
        assert_eq!(
            fresh_get_on_reason(true, 0),
            Some(1),
            "an unknown reason takes the left clip, not the riding loops"
        );
        assert_eq!(
            fresh_get_on_reason(false, 2),
            None,
            "a rider that streams in already seated starts in the riding loops"
        );
    }

    #[test]
    fn the_get_on_one_shot_falls_into_the_riding_loop() {
        let mut anim = RiderAnim::new(durations(), RidePhase::Mounting);
        assert_eq!(
            anim.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::GetOn)
        );
        assert_eq!(
            anim.advance(0.5, 0.0, 9.0, true),
            RideStep::None,
            "still getting on"
        );
        assert_eq!(
            anim.advance(0.5, 0.0, 9.0, true),
            RideStep::Play(RiderClip::Idle)
        );
        assert_eq!(
            anim.advance(0.1, 0.0, 9.0, true),
            RideStep::None,
            "the idle loops"
        );
        // without a get-on clip the riding loop starts at once
        let mut none = RiderAnim::new(RiderDurations::default(), RidePhase::Mounting);
        assert_eq!(
            none.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::Idle)
        );
    }

    #[test]
    fn a_rider_first_seen_mounted_starts_riding_without_a_get_on() {
        let seen = Seen::default();
        let attached = Some((5, 1));
        assert!(
            !fresh_attach(true, attached, &seen, 7),
            "the first sight of a unit that is already attached is not a get-on"
        );
        let mut streamed = riding();
        assert_eq!(
            streamed.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::Idle),
            "a rider that streams in already riding does not replay the get-on"
        );
        let mut fresh = RiderAnim::new(durations(), RidePhase::Mounting);
        assert_eq!(
            fresh.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::GetOn)
        );
    }

    #[test]
    fn a_rider_seen_on_foot_gets_on_when_it_attaches() {
        let mut seen = Seen::default();
        seen.observe(7, None, false);
        let attached = Some((5, 1));
        assert!(fresh_attach(true, attached, &seen, 7), "on foot, then attached");
        assert_eq!(
            fresh_get_on_reason(fresh_attach(true, attached, &seen, 7), 2),
            Some(2)
        );
        seen.observe(7, attached, true);
        assert!(!fresh_attach(true, attached, &seen, 7), "the seat is recorded");
    }

    #[test]
    fn a_rider_that_leaves_the_view_mounted_returns_seated() {
        let mut seen = Seen::default();
        let attached = Some((5, 1));
        seen.observe(7, attached, true);
        // the entity is gone while it is out of range; the map keeps the seat it was last seen on
        assert!(
            !fresh_attach(true, attached, &seen, 7),
            "a returning rider would replay the get-on"
        );
        assert_eq!(fresh_get_on_reason(false, 2), None);
    }

    #[test]
    fn a_unit_seen_on_foot_that_returns_mounted_does_not_get_on_again() {
        let mut seen = Seen::default();
        seen.observe(7, None, false);
        // it walks out of view (no entity), mounts there and rides back in
        seen.forget_absent(&std::collections::HashSet::new());
        assert!(!fresh_attach(true, Some((5, 1)), &seen, 7));
    }

    #[test]
    fn waiting_for_the_animator_or_mount_data_keeps_a_seen_on_foot_attach_fresh() {
        let mut seen = Seen::default();
        seen.observe(7, None, false);
        // the attach arrives but Rider::bind waits for the animator or the mount's unit
        seen.observe(7, Some((5, 1)), false);
        assert!(
            fresh_attach(true, Some((5, 1)), &seen, 7),
            "the wait does not lose the get-on"
        );
        // a unit first seen attached with no data yet is not fresh once the data arrives
        let mut first = Seen::default();
        first.observe(8, Some((5, 1)), false);
        assert!(
            !fresh_attach(true, Some((5, 1)), &first, 8),
            "a seated rider does not turn into a fresh get-on"
        );
    }

    #[test]
    fn the_gait_bands_play_the_idle_to_run_and_run_to_idle_one_shots() {
        let mut anim = mounted();
        // PseudoSpeed = speed / the mount's 9 m/s run: 3 m/s is 0.33 (DriverWalk), 9 m/s is 1 (DriverRun)
        assert_eq!(
            anim.advance(0.0, 3.0, 9.0, true),
            RideStep::Play(RiderClip::Walk)
        );
        assert_eq!(
            anim.advance(0.0, 9.0, 9.0, true),
            RideStep::Play(RiderClip::Run),
            "walk to run fades directly"
        );
        assert_eq!(
            anim.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::RunToIdle)
        );
        assert_eq!(
            anim.advance(0.4, 0.0, 9.0, true),
            RideStep::None,
            "the run-to-idle clip is 0.45 s long, not the 0.2 s blend"
        );
        assert_eq!(
            anim.advance(0.05, 0.0, 9.0, true),
            RideStep::Play(RiderClip::Idle)
        );
        assert_eq!(
            anim.advance(0.0, 9.0, 9.0, true),
            RideStep::Play(RiderClip::IdleToRun)
        );
        assert_eq!(
            anim.advance(0.5, 9.0, 9.0, true),
            RideStep::None,
            "the idle-to-run clip is 0.6 s long"
        );
        assert_eq!(anim.advance(0.1, 9.0, 9.0, true), RideStep::None);
        assert_eq!(
            anim.advance(0.01, 9.0, 9.0, true),
            RideStep::Play(RiderClip::Run)
        );
    }

    #[test]
    fn a_speed_change_during_a_transition_is_picked_up_at_its_end() {
        let mut anim = riding();
        assert_eq!(
            anim.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::Idle)
        );
        assert_eq!(
            anim.advance(0.0, 9.0, 9.0, true),
            RideStep::Play(RiderClip::IdleToRun)
        );
        assert_eq!(
            anim.advance(0.3, 0.0, 9.0, true),
            RideStep::None,
            "the one-shot finishes even though the mount stopped"
        );
        assert_eq!(
            anim.advance(0.3, 0.0, 9.0, true),
            RideStep::Play(RiderClip::Idle),
            "the gait is chosen again at the end, not the stale Run"
        );
        assert_eq!(
            anim.advance(0.0, 9.0, 9.0, true),
            RideStep::Play(RiderClip::IdleToRun)
        );
        assert_eq!(
            anim.advance(0.6, 9.0, 9.0, true),
            RideStep::Play(RiderClip::Run)
        );
        assert_eq!(
            anim.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::RunToIdle)
        );
        assert_eq!(
            anim.advance(0.2, 9.0, 9.0, true),
            RideStep::None,
            "the mount runs again before the run-to-idle clip ends"
        );
        assert_eq!(
            anim.advance(0.25, 9.0, 9.0, true),
            RideStep::Play(RiderClip::Run)
        );
    }

    #[test]
    fn the_gait_maps_the_graphs_pseudo_speed_bands_to_the_mounts_run_speed() {
        assert_eq!(gait(0.0, 9.0, true), Gait::Idle);
        assert_eq!(gait(0.3, 9.0, true), Gait::Idle, "DriverIdle's 0.1");
        assert_eq!(
            gait(1.0, 9.0, true),
            Gait::Idle,
            "the 0.1..0.15 gap keeps the lower state"
        );
        assert_eq!(gait(1.4, 9.0, true), Gait::Walk, "DriverWalk's 0.15");
        assert_eq!(gait(3.0, 9.0, true), Gait::Walk);
        assert_eq!(gait(5.4, 9.0, true), Gait::Walk, "DriverWalk's 0.6");
        assert_eq!(gait(6.4, 9.0, true), Gait::Run, "DriverRun's 0.7");
        assert_eq!(
            gait(9.0, 9.0, true),
            Gait::Run,
            "PseudoSpeed 1 at the mount's run speed"
        );
        assert_eq!(
            gait(18.0, 9.0, true),
            Gait::Run,
            "DriverRun's 2, a speed buff"
        );
        assert_eq!(gait(7.2, 7.2, true), Gait::Run, "a donkey's own run speed");
        assert_eq!(gait(9.0, 9.0, false), Gait::Walk, "no run clip");
    }

    #[test]
    fn the_gait_keeps_the_lower_state_in_the_band_gaps() {
        assert_eq!(gait(1.0, 9.0, true), Gait::Idle, "PseudoSpeed 0.111");
        assert_eq!(gait(1.34, 9.0, true), Gait::Idle, "PseudoSpeed 0.149");
        assert_eq!(gait(1.36, 9.0, true), Gait::Walk, "PseudoSpeed 0.151");
        assert_eq!(gait(6.2, 9.0, true), Gait::Walk, "PseudoSpeed 0.689");
        assert_eq!(gait(6.3, 9.0, true), Gait::Run, "PseudoSpeed 0.7");
        let mut anim = mounted();
        assert_eq!(
            anim.advance(0.0, 3.0, 9.0, true),
            RideStep::Play(RiderClip::Walk)
        );
        assert_eq!(
            anim.advance(0.0, 9.0, 9.0, true),
            RideStep::Play(RiderClip::Run),
            "walk to run fades directly"
        );
        assert_eq!(
            anim.advance(0.0, 6.2, 9.0, true),
            RideStep::Play(RiderClip::Walk),
            "the 0.6..0.7 gap keeps the walk, not the idle"
        );
        assert_eq!(
            anim.advance(0.0, 1.0, 9.0, true),
            RideStep::Play(RiderClip::Idle),
            "the 0.1..0.15 gap keeps the idle"
        );
    }

    #[test]
    fn getting_off_plays_start_loop_and_end_then_walks() {
        let mut anim = mounted();
        assert!(anim.begin_dismount());
        assert_eq!(
            anim.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::GetOffStart)
        );
        assert_eq!(anim.advance(0.66, 0.0, 9.0, true), RideStep::None);
        assert_eq!(
            anim.advance(0.01, 0.0, 9.0, true),
            RideStep::Play(RiderClip::GetOffLoop)
        );
        assert_eq!(
            anim.advance(0.5, 0.0, 9.0, true),
            RideStep::Play(RiderClip::GetOffEnd)
        );
        assert_eq!(anim.advance(0.82, 0.0, 9.0, true), RideStep::None);
        assert_eq!(anim.advance(0.01, 0.0, 9.0, true), RideStep::OnFoot);
        // a mount with no get-off clips walks at once
        let mut none = RiderAnim::new(RiderDurations::default(), RidePhase::Mounting);
        assert!(!none.begin_dismount());
        // a detach while the get-on plays replaces it with the get-off chain
        let mut early = RiderAnim::new(durations(), RidePhase::Mounting);
        assert_eq!(
            early.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::GetOn)
        );
        assert!(early.begin_dismount());
        assert_eq!(
            early.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::GetOffStart)
        );
    }

    #[test]
    fn a_mount_that_leaves_mid_animation_still_finishes_the_one_shot() {
        // pose_riders steps a rider whose mount despawned with speed 0; the get-on still ends in the idle
        let mut anim = RiderAnim::new(durations(), RidePhase::Mounting);
        assert_eq!(
            anim.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::GetOn)
        );
        assert_eq!(anim.advance(0.6, 0.0, 9.0, true), RideStep::None);
        assert_eq!(
            anim.advance(0.4, 0.0, 9.0, true),
            RideStep::Play(RiderClip::Idle)
        );
    }

    #[test]
    fn a_remount_while_getting_off_seats_again() {
        assert_eq!(
            seat_action(Some((5, 1)), Some((5, 1, false))),
            SeatAction::None
        );
        assert_eq!(
            seat_action(Some((5, 1)), Some((5, 1, true))),
            SeatAction::Remount,
            "attached again while getting off"
        );
        assert_eq!(
            seat_action(Some((5, 1)), Some((6, 1, false))),
            SeatAction::Remount,
            "another mount"
        );
        assert_eq!(
            seat_action(Some((5, 1)), Some((5, 2, true))),
            SeatAction::Remount,
            "another seat"
        );
        assert_eq!(seat_action(Some((5, 1)), None), SeatAction::Mount);
        assert_eq!(seat_action(None, Some((5, 1, false))), SeatAction::Dismount);
        assert_eq!(
            seat_action(None, Some((5, 1, true))),
            SeatAction::None,
            "the get-off plays out before anything else"
        );
        assert_eq!(seat_action(None, None), SeatAction::None);
        // the rebind starts in the riding loops, not with the get-on
        let mut anim = riding();
        assert_eq!(
            anim.advance(0.0, 0.0, 9.0, true),
            RideStep::Play(RiderClip::Idle)
        );
    }

    #[test]
    fn non_horse_mounts_keep_their_own_clips_and_only_the_graphs_states() {
        assert_eq!(
            MountKind::of(0),
            MountKind::Horse,
            "a donkey, an elk, a yata"
        );
        assert_eq!(MountKind::of(1), MountKind::Horse);
        assert_eq!(
            MountKind::of(5),
            MountKind::Horse,
            "a submarine has no rider states"
        );
        assert!(rider_states(0), "a donkey still gets the horse states");
        assert!(rider_states(1));
        assert!(
            !rider_states(5),
            "the boats and the car keep the old loop-only rider"
        );
        assert!(!rider_states(8));
        assert_eq!(MountKind::of(2).idle(), "lion_ba_relaxed_idle");
        assert_eq!(MountKind::of(2).walk(), "lion_mo_normal_walk_f");
        assert_eq!(
            MountKind::of(2).run(),
            "_FIST_MO_RIDER_LION_RUN",
            "Lion_DriverRun :1615"
        );
        assert_eq!(
            MountKind::of(4).idle(),
            "lion_ba_relaxed_idle",
            "Wildboar_DriverIdle :1839"
        );
        assert_eq!(MountKind::of(4).run(), "_FIST_MO_RIDER_WILDBOAR_RUN");
        assert_eq!(
            MountKind::of(3).walk(),
            "horse_mo_normal_walk_f",
            "Bear_DriverWalk :1315"
        );
        assert_eq!(MountKind::of(3).run(), "_FIST_MO_RIDER_LION_RUN");
        assert_eq!(
            MountKind::of(9).run(),
            "_FIST_MO_RIDER_HORSE_RUN",
            "Elephant_DriverRun :1394"
        );
        assert_eq!(
            MountKind::of(9).idle_to_run(),
            "_FIST_MO_RIDER_ELEPHANT_IDLETORUN"
        );
        assert_eq!(MountKind::of(10).run(), "_FIST_MO_RIDER_PANGOLIN_RUN");
        assert_eq!(
            MountKind::of(10).idle_to_run(),
            "_FIST_MO_RIDER_HORSE_IDLETORUN",
            "Pangolin_DriverIdleToRun :1693"
        );
        assert_eq!(
            MountKind::of(9).idle_to_run_fallback(),
            None,
            "no elephant idle-to-run clip in 1.2: a direct crossfade, not the horse's"
        );
        assert_eq!(
            MountKind::of(9).run_to_idle(),
            None,
            "no elephant run-to-idle clip in 1.2"
        );
        assert_eq!(
            MountKind::of(2).run_to_idle(),
            Some("lion_mo_relaxed_runtoidle_f")
        );
        assert_eq!(
            MountKind::of(10).idle_to_run_fallback(),
            Some("horse_mo_relaxed_idletorun_f")
        );
    }

    #[test]
    fn the_forward_example_is_the_groups_f_clip() {
        let lmg = aa_anim::Lmg::parse(
            r#"<LocomotionGroup><ExampleList>
                <Example Position="-1, 1, 0" AName="horse_mo_relaxed_idletorun_f" />
                <Example Position=" 1, 1, 0" AName="horse_mo_normal_run_f" />
                <Example Position=" 0, 1, 0" AName="horse_mo_normal_run_f" />
                <Example Position=" 0, 1, 0" AName="horse_mo_normal_runuphill" />
            </ExampleList></LocomotionGroup>"#,
        )
        .unwrap();
        assert_eq!(
            forward_example(&lmg).map(|e| e.animation.as_str()),
            Some("horse_mo_normal_run_f"),
            "not the idle-to-run clip listed first"
        );
        let lion = aa_anim::Lmg::parse(
            r#"<LocomotionGroup><ExampleList>
                <Example Position=" 1, 1, 0" AName="lion_mo_normal_run_f" />
                <Example Position="-1, 1, 0" AName="lion_mo_normal_run_fl" />
                <Example Position=" 0, 1, 0" AName="lion_mo_normal_runuphill" />
            </ExampleList></LocomotionGroup>"#,
        )
        .unwrap();
        assert_eq!(
            forward_example(&lion).map(|e| e.animation.as_str()),
            Some("lion_mo_normal_run_f"),
            "the uphill clip sits on the forward axis"
        );
    }

    #[test]
    fn a_mount_reason_only_applies_to_a_driver_seat() {
        let mut reason = MountReason(3);
        assert_eq!(reason.pending(1, 1), Some(3));
        assert_eq!(reason.0, 3, "kept until the bind succeeds");
        reason.used();
        assert_eq!(reason.0, 0);
        let mut passenger = MountReason(3);
        assert_eq!(passenger.pending(2, 1), None, "a passenger seat");
        assert_eq!(passenger.0, 0, "drops the reason of the refused mount");
        let mut boat = MountReason(3);
        assert_eq!(boat.pending(1, 6), None, "a boat has no rider states");
        assert_eq!(boat.0, 0);
        let mut none = MountReason(0);
        assert_eq!(none.pending(1, 1), None);
        let mut bad = MountReason(9);
        assert_eq!(bad.pending(1, 1), None);
        assert_eq!(bad.0, 0, "an out-of-range reason is dropped");
    }

    /// The 1.2 `.cal` names every get-on, gait and get-off clip the graph uses, and its `_FIST_MO_RIDER_*_RUN`
    /// locomotion groups resolve to the forward clips.
    #[test]
    #[ignore = "needs AA_PAK"]
    fn the_1_2_cal_has_the_rider_clips_and_the_run_groups_resolve() {
        if !aa_cry::pak_available() {
            eprintln!("no pak: skipped");
            return;
        }
        let cal =
            Cal::shared("objects/characters/nuian/male/nude/nu_m_base.cal").expect("the .cal");
        let rig = aa_anim::pose::shared_rig("objects/characters/nuian/male/nude/nu_m_base.chr")
            .expect("the rig");
        let clips = ClipLibrary::default();
        let bind = |name: &str| bind_rider_clip(&rig, &cal, &clips, name);
        for kind in [
            MountKind::Horse,
            MountKind::Lion,
            MountKind::Bear,
            MountKind::Wildboar,
            MountKind::Pangolin,
            MountKind::Elephant,
        ] {
            for reason in 1..=4 {
                let name = get_on_clip(kind, reason);
                assert!(bind(&name).is_some(), "{kind:?} reason {reason}: {name}");
            }
            assert!(bind(kind.idle()).is_some(), "{kind:?}: {}", kind.idle());
            assert!(bind(kind.walk()).is_some(), "{kind:?}: {}", kind.walk());
            let run = bind(kind.run()).unwrap_or_else(|| panic!("{kind:?}: {}", kind.run()));
            let forward = format!("/{}.caf", kind.run_fallback());
            assert!(
                run.source.as_deref().is_some_and(|s| s.ends_with(&forward)),
                "{kind:?}: {:?}",
                run.source
            );
            // the group's forward example (not the one listed first) is the idle-to-run clip
            if let Some(name) = kind.idle_to_run_fallback() {
                let clip = bind(kind.idle_to_run())
                    .unwrap_or_else(|| panic!("{kind:?}: {}", kind.idle_to_run()));
                let forward = format!("/{name}.caf");
                assert!(
                    clip.source
                        .as_deref()
                        .is_some_and(|s| s.ends_with(&forward)),
                    "{kind:?}: {:?}",
                    clip.source
                );
            } else {
                assert!(
                    bind(kind.idle_to_run()).is_none(),
                    "{kind:?} has no 1.2 idle-to-run clip"
                );
            }
            if let Some(name) = kind.run_to_idle() {
                let clip = bind(name).unwrap_or_else(|| panic!("{kind:?}: {name}"));
                let forward = format!("/{name}.caf");
                assert!(
                    clip.source
                        .as_deref()
                        .is_some_and(|s| s.ends_with(&forward)),
                    "{kind:?}: {:?}",
                    clip.source
                );
            } else {
                assert_eq!(kind, MountKind::Elephant);
            }
        }
        assert!(
            bind(MountKind::Elephant.idle_to_run()).is_none(),
            "the 1.2 .cal has no elephant idle-to-run group"
        );
        for name in [GET_OFF_START, GET_OFF_LOOP, GET_OFF_END] {
            assert!(bind(name).is_some(), "{name}");
        }
    }

    #[test]
    fn only_the_driver_s_seat_steers() {
        let seat = |point| {
            LocalMount(Some(LocalSeat {
                mount: 7,
                point,
                drives: point == 1,
                cart: false,
                speed: MountSpeed::default(),
                lift: 1.2,
            }))
        };
        assert_eq!(seat(1).driving(), Some((7, MountSpeed::default())));
        assert!(!seat(1).passenger());
        assert_eq!(seat(1).cart_driver(), None);
        assert_eq!(seat(2).driving(), None);
        assert!(seat(2).passenger());
        let cart = |point| {
            LocalMount(Some(LocalSeat {
                mount: 7,
                point,
                drives: point == 1,
                cart: true,
                speed: MountSpeed::default(),
                lift: 1.2,
            }))
        };
        assert_eq!(cart(1).cart_driver(), Some(7));
        assert_eq!(cart(1).driving(), None, "a cart has its own controller");
        assert_eq!(cart(2).cart_driver(), None);
        assert!(cart(2).passenger());
        assert!((seat(2).lift() - 0.35).abs() < 1e-6, "the seat 1.2 m up, less the seated drop");
        assert_eq!(LocalMount::default().driving(), None);
        assert_eq!(LocalMount::default().lift(), 0.0);
    }

    /// A rider of the test durations on an empty skeleton, and the animator [`Rider::step`] drives; the clip names
    /// are the [`RiderClip`] variants in snake case.
    fn stepped_rider(anim: RiderAnim) -> (Rider, Animator) {
        let rig = std::sync::Arc::new(Rig::new(aa_cry::Skeleton::default()));
        let bound = |name: &str, seconds: f32| {
            BoundClip::new(
                &rig,
                std::sync::Arc::new(aa_anim::Clip {
                    name: name.to_string(),
                    duration: seconds,
                    ..Default::default()
                }),
                aa_anim::SampleOptions::default(),
            )
        };
        let d = durations();
        let clips = RiderClips {
            get_on: d.get_on.map(|s| bound("get_on", s)),
            idle: bound("idle", 1.0),
            walk: Some(bound("walk", 1.0)),
            run: Some(bound("run", 1.0)),
            idle_to_run: d.idle_to_run.map(|s| bound("idle_to_run", s)),
            run_to_idle: d.run_to_idle.map(|s| bound("run_to_idle", s)),
            get_off_start: d.get_off_start.map(|s| bound("get_off_start", s)),
            get_off_loop: d.get_off_loop.map(|s| bound("get_off_loop", s)),
            get_off_end: d.get_off_end.map(|s| bound("get_off_end", s)),
        };
        let rider = Rider {
            unit: 7,
            mount: 5,
            point: 1,
            clips,
            anim,
            run_speed: 9.0,
            on_foot: None,
        };
        (rider, Animator::new(rig, Vec::new()))
    }

    fn playing(animator: &Animator) -> &str {
        animator
            .current
            .as_ref()
            .map_or("-", |p| p.clip.clip.name.as_str())
    }

    #[test]
    fn a_frame_held_animator_pauses_the_get_on_one_shot() {
        let (mut rider, mut animator) =
            stepped_rider(RiderAnim::new(durations(), RidePhase::Mounting));
        animator.set_framehold(true);
        assert!(!rider.step(&mut animator, 0.25, 0.0, 1.0));
        assert_eq!(playing(&animator), "-");
        assert!(
            !rider.anim.entered,
            "the get-on did not start behind the hold"
        );
        assert_eq!(rider.anim.left, 0.0, "no timer ran behind the hold");
        // the release starts it from the beginning, as the animator's clock did
        animator.set_framehold(false);
        assert!(!rider.step(&mut animator, 0.0, 0.0, 1.0));
        assert_eq!(playing(&animator), "get_on");
        assert!(!rider.step(&mut animator, 0.5, 0.0, 1.0));
        assert!((rider.anim.left - 0.5).abs() < 1e-6);
        // held half way: the timer stops with the clock
        animator.set_framehold(true);
        assert!(!rider.step(&mut animator, 2.0, 0.0, 1.0));
        assert_eq!(playing(&animator), "get_on");
        assert!((rider.anim.left - 0.5).abs() < 1e-6);
        assert_eq!(rider.anim.phase, RidePhase::Mounting);
        // unfrozen: it continues from the 0.5 s that was left, not from the start
        animator.set_framehold(false);
        assert!(!rider.step(&mut animator, 0.25, 0.0, 1.0));
        assert!((rider.anim.left - 0.25).abs() < 1e-6);
        assert_eq!(rider.anim.phase, RidePhase::Mounting);
        assert!(!rider.step(&mut animator, 0.25, 0.0, 1.0));
        assert_eq!(rider.anim.phase, RidePhase::Riding);
        assert_eq!(playing(&animator), "idle");
    }

    #[test]
    fn a_frame_held_animator_does_not_finish_the_get_off() {
        let (mut rider, mut animator) =
            stepped_rider(RiderAnim::new(durations(), RidePhase::Riding));
        assert!(rider.anim.begin_dismount());
        assert!(!rider.step(&mut animator, 0.0, 0.0, 1.0));
        assert_eq!(playing(&animator), "get_off_start");
        assert!(!rider.step(&mut animator, 0.5, 0.0, 1.0));
        animator.set_framehold(true);
        for _ in 0..20 {
            assert!(
                !rider.step(&mut animator, 0.2, 0.0, 1.0),
                "a held animator keeps the rider seated"
            );
        }
        assert!((rider.anim.left - 0.17).abs() < 1e-6);
        assert!(rider.dismounting());
        assert_eq!(playing(&animator), "get_off_start");
        animator.set_framehold(false);
        assert!(!rider.step(&mut animator, 0.2, 0.0, 1.0));
        assert_eq!(playing(&animator), "get_off_loop");
        assert!(!rider.step(&mut animator, 0.5, 0.0, 1.0));
        assert_eq!(playing(&animator), "get_off_end");
        assert!(
            rider.step(&mut animator, 0.83, 0.0, 1.0),
            "only after its clips ran does the unit walk"
        );
    }
}
