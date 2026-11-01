//! Ladders: the use trigger, the climb, the top and bottom exits, the hang movement packets and
//! the climb clips.
//!
//! 1.2 finds the doodad under the use trace, builds its ladder frame (`FUN_390c1130` with the
//! `$ladder` element read by `FUN_393aaad0`) and calls `Player::GrabOnLadder` `FUN_391388f0` with
//! reason 10. `GrabOnLadder` stores the doodad id, the climb kind and the frame's climb distance
//! (`Player+0x244`, the climbData x) and relative angle (`+0x24c..0x258`); the values are constant
//! for the whole hang (`FUN_39137470` only reads them). The per-frame placement keeps the body on
//! the ladder.
//!
//! This module mirrors the settled parts:
//! * the climb kind comes from `doodad_funcs` -> `doodad_func_climbs` for the doodad's current
//!   phase and the frame from the model's `$ladder` element (`Db::ladder_kind`,
//!   [`super::doodad_model::LadderHelper`]);
//! * F (`do_interaction_1`) grabs the nearest climbable doodad within 1.5 m (the reach
//!   `FUN_390c1130` uses without an actor-model reach block); the local body is placed at once,
//!   before the server answers, and `CSHang` is sent (`FUN_39363410`);
//! * W/S move along the ladder. 1.2 has no climb-speed constant: the vertical motion is the
//!   clip's root motion, scaled by the buffs' `ladder_speed_mul` (unit attribute 93 [0x5d]). The
//!   clip's root speed is the fallback when the model has no climb clips (1.5 m/s,
//!   capture-pending);
//! * `CSMoveUnit` carries actor flag `0x40`, gcId 1 and the [`aa_net::climb`] data of the grab
//!   (constant). On a ladder of a parented doodad (a ship's side ladder) it also carries the
//!   `0x20` block with the hull id, the hull-local primary position and the world backup;
//! * reaching the top starts the exit (local reason 2, no climb bit for its duration; after 0.5 s
//!   the body steps onto the platform and `CSUnhang` 1 is sent), the bottom stands again
//!   (`CSUnhang` 1), F lets go (1), Space jumps forward (6, the mode-6 velocities), a vanished
//!   ladder releases with reason 0 (the 1.2 "my ladder is missing" path) and death releases with
//!   reason 0xb (`FUN_39137320`, the only native 0xb site).
//!
//! Capture-pending (not settled by the decompile): which key produces each of the exit modes
//! 3-9; the exact `deltaMovement` and velocity while climbing; the lateral `y` of the climb data
//! on nets and guardrails (this client climbs centred, `y = 0`); the top step distance and the
//! camera pitch clamp; and how a remote client animates a hanger (the client packet consumer was
//! not traced). The remote side here uses the `SCHung` target's kind and the climb data.

use aa_anim::Animator;
use aa_net::climb;
use aa_net::model::{MoveSupport, radians_to_direction, stance};
use aa_net::state::{Doodads, Units};
use aa_net::{Command, MoveCommand, MoveState, NetCommand, Pos};
use aa_world::{TerrainQuery, WaterKind, WaterQuery, WorldCollision, bevy_to_cry};
use bevy::prelude::*;

use super::anim::{ClimbPose, Locomotion, Motion};
use super::data::Db;
use super::doodad_model::LadderHelper;
use super::doodad_visual::DoodadVisual;
use super::player::{self, LocalPlayer, Mover, PlayerInput, SendState};
use super::swim::{ActorParams, Swim, Wet};
use super::units::{DeckHullPoses, RemoteDoodad, Views};

/// The reach of the use trace without an actor-model reach block (1.5 m, `FUN_390c1130`).
pub const REACH: f32 = 1.5;
/// The capsule radius the body is held off the ladder by (m).
pub const STANDOFF: f32 = 0.35;
/// Climb-speed fallback when the model has no climb clips (m/s; capture-pending, the 1.2 speed is
/// the clip's root motion).
pub const FALLBACK_CLIMB_SPEED: f32 = 1.5;
/// The top exit keeps the body on the ladder for this long before leaving (`FUN_391387c0`: 0.5 s).
pub const TOP_EXIT_SECS: f32 = 0.5;
/// How far the body steps over the top on the way out (m; capture-pending).
pub const TOP_STEP: f32 = 0.6;
/// The mode-6 forward jump (`FUN_39135570`: 4.5 m/s forward, 7.0 m/s up).
const JUMP_FORWARD: f32 = 4.5;
const JUMP_UP: f32 = 7.0;
/// How far the bottom exit snaps down (or up) to the ground before it counts as a fall (m).
const BOTTOM_SNAP: f32 = 0.6;

const EPS: f32 = 0.01;

/// F is claimed for a ladder in reach that the grab gate accepts (or the one being climbed), so
/// NPC talk, loot-all and the doodad bar do not fire on the same press. `Some(doodad)` names the
/// ladder.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LadderClaim(pub Option<u32>);

/// The optional world state the ladder controller reads (grouped to stay inside Bevy's
/// system-parameter limit).
#[derive(bevy::ecs::system::SystemParam)]
pub struct LadderWorld<'w> {
    pub mount: Option<Res<'w, super::mount::LocalMount>>,
    pub gliding: Option<Res<'w, super::glide::Gliding>>,
    pub terrain: Option<Res<'w, TerrainQuery>>,
    pub collision: Option<Res<'w, WorldCollision>>,
    pub water: Option<Res<'w, WaterQuery>>,
    pub db: Option<Res<'w, Db>>,
    pub views: Option<Res<'w, Views>>,
    pub hull_poses: Option<Res<'w, DeckHullPoses>>,
    pub sprint: Option<Res<'w, super::sprint::Sprint>>,
    pub claim: ResMut<'w, LadderClaim>,
}

/// The ladder the local player hangs on.
#[derive(Component, Clone, Debug)]
pub struct Ladder {
    /// The climb doodad (`Player+0x238`).
    pub obj_id: u32,
    /// `doodad_func_climbs.climb_type_id` (`Player+0x23c`).
    pub kind: u8,
    /// The doodad's parent unit (a ship's side ladder), 0 for a static doodad.
    pub parent: u32,
    /// The doodad's attach point on the parent (`gcPartId` of the `0x20` block).
    pub attach_point: u8,
    /// The `$ladder` helper in the doodad root's local space.
    pub helper: LadderHelper,
    /// The outward normal in the doodad root's local space (from the ladder to the climber).
    pub normal: Vec3,
    /// Position along the climb, 0 at the bottom and 1 at the top.
    pub fraction: f32,
    /// The 32-bit climb data of the grab (constant for the hang).
    pub climb_data: u32,
    /// The ladder's angle from vertical (degrees), for the top exit class.
    pub angle_deg: f32,
    /// Metres per second along the ladder (the clip's root speed times `ladder_speed_mul`).
    pub speed: f32,
    /// A top or bottom exit is playing.
    pub exiting: Option<Exit>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exit {
    /// Seconds left before the body leaves.
    pub left: f32,
    /// The exit is at the top (steps over) rather than at the bottom (stands).
    pub top: bool,
}

/// Why the local player left (`CSUnhang` reason, `Player::LeaveLadder`).
pub mod reason {
    /// The ladder doodad disappeared.
    pub const LADDER_MISSING: i32 = 0;
    /// A generic release / the end of the top exit.
    pub const RELEASE: i32 = 1;
    /// The forward jump off (`FUN_39135570` mode 6).
    pub const FORWARD_JUMP: i32 = 6;
    /// The unit died while hanging: the value-change callback (`FUN_39137320`) leaves with 0xb
    /// when the value reaches 0. It is the only native 0xb site (no release on mount, skill,
    /// glide or stance).
    pub const LADDER_LOST: i32 = 0xb;
}

/// The clip family of a climb kind.
///
/// From `player_full_body.xml`: kinds 1, 6, 9 and 10 (big tree) use the `fist_mo_ladder_*`
/// states, and so does the net (kind 4: `net_up/idle/down` play `fist_mo_ladder_*`); kind 2
/// (rope) plays `fist_mo_rope_up`, kind 3 (wood) `fist_mo_wood_up`, kind 5 (horizontal rope)
/// the `fist_mo_hrope_*` states (`fist_mo_rope_front`/`_front_clip`/`_back_clip`/`_clip_idle`),
/// kind 7 the mast and kind 8 the guardrail (`fist_mo_climb_*`). `fist_mo_prope_*` is the
/// Vehicle prope, not a climb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Ladder,
    Wood,
    Rope,
    Hrope,
    Mast,
    Guardrail,
}

impl Family {
    pub fn of(kind: u8) -> Family {
        match kind {
            2 => Family::Rope,
            3 => Family::Wood,
            5 => Family::Hrope,
            7 => Family::Mast,
            8 => Family::Guardrail,
            _ => Family::Ladder,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Family::Ladder => "ladder",
            Family::Wood => "wood",
            Family::Rope => "rope",
            Family::Hrope => "rope",
            Family::Mast => "mast",
            Family::Guardrail => "climb",
        }
    }
}

/// The climb and exit clips of one model.
#[derive(Clone)]
pub struct LadderClips {
    ladder: FamilyClips,
    wood: FamilyClips,
    rope: FamilyClips,
    hrope: FamilyClips,
    mast: FamilyClips,
    guardrail: FamilyClips,
    ladder_up_45: Option<aa_anim::pose::BoundClip>,
    exit_ladder: Option<aa_anim::pose::BoundClip>,
    exit_ladder_80: Option<aa_anim::pose::BoundClip>,
    exit_45: Option<aa_anim::pose::BoundClip>,
    exit_70: Option<aa_anim::pose::BoundClip>,
    exit_rope: Option<aa_anim::pose::BoundClip>,
}

impl LadderClips {
    /// Binds the climb clips from the model's `.cal` (which names them through its includes; some
    /// are aliases of another file, e.g. `fist_mo_wood_up` is the ladder up clip) and from the
    /// folder of the model's `fist_` clips for a name the cal lacks.
    pub fn load(
        rig: &aa_anim::pose::Rig,
        cal: &aa_anim::Cal,
        clips: &aa_anim::ClipLibrary,
    ) -> Option<LadderClips> {
        let folder = cal
            .animation("fist_ba_relaxed_idle")
            .or_else(|| cal.animations.values().find(|e| e.path.contains("/fist/")))
            .or_else(|| cal.animations.values().next())?
            .path
            .rsplit_once('/')?
            .0
            .to_string();
        let bind = |name: &str| bind_path(rig, cal, clips, &folder, name);
        let family = |f: Family| FamilyClips::load(rig, cal, clips, &folder, f);
        let mut set = LadderClips {
            ladder: family(Family::Ladder),
            wood: family(Family::Wood),
            rope: family(Family::Rope),
            hrope: family(Family::Hrope),
            mast: family(Family::Mast),
            guardrail: family(Family::Guardrail),
            ladder_up_45: bind("fist_mo_ladder_up_45"),
            exit_ladder: bind("fist_mo_ladder_end"),
            exit_ladder_80: bind("fist_mo_ladder_end_80"),
            exit_45: bind("fist_mo_ladder_end_45"),
            exit_70: bind("fist_mo_ladder_end_70"),
            exit_rope: bind("fist_mo_rope_end"),
        };
        // a family without its own clips (a model that lacks them) climbs like a ladder
        for f in [
            &mut set.wood,
            &mut set.rope,
            &mut set.hrope,
            &mut set.mast,
            &mut set.guardrail,
        ] {
            if f.idle.is_none() {
                *f = set.ladder.clone();
            }
        }
        set.ladder.idle.as_ref()?;
        Some(set)
    }

    fn family(&self, kind: u8) -> &FamilyClips {
        match Family::of(kind) {
            Family::Ladder => &self.ladder,
            Family::Wood => &self.wood,
            Family::Rope => &self.rope,
            Family::Hrope => &self.hrope,
            Family::Mast => &self.mast,
            Family::Guardrail => &self.guardrail,
        }
    }

    /// The up clip of a kind (the 45-degree ladder has its own when the model has one).
    pub fn up(&self, kind: u8) -> Option<&aa_anim::pose::BoundClip> {
        if kind == 9 && self.ladder_up_45.is_some() {
            self.ladder_up_45.as_ref()
        } else {
            self.family(kind).up.as_ref()
        }
    }

    /// The standing clip on the ladder.
    pub fn idle(&self, kind: u8) -> Option<&aa_anim::pose::BoundClip> {
        self.family(kind).idle.as_ref()
    }

    /// The down clip (the rope and wood families have none: their up clip plays backwards).
    pub fn down(&self, kind: u8) -> Option<&aa_anim::pose::BoundClip> {
        self.family(kind)
            .down
            .as_ref()
            .or(self.family(kind).up.as_ref())
    }

    /// The top exit clip by the ladder's angle class (`FUN_3916c1f0/c280`): kind 1/6 below 5
    /// degrees `exit_ladder_top`, below 35 `exit_70_ladder_top`, below 50 `exit_45_ladder_top`;
    /// kind 9 `exit_45_ladder_top`; nets `climb_net` below 1 degree and `climb_net_80` below 20
    /// (both play `fist_mo_ladder_end[_80]`); the rope uses `fist_mo_rope_end`; the guardrail's
    /// `exist_guardrail_top` state plays `fist_mo_climb_up`; the wood wall, masts and the big tree
    /// have none.
    pub fn exit(&self, kind: u8, angle_deg: f32) -> Option<&aa_anim::pose::BoundClip> {
        match kind {
            9 => self.exit_45.as_ref().or(self.exit_ladder.as_ref()),
            1 | 6 => {
                if angle_deg < 5.0 {
                    self.exit_ladder.as_ref()
                } else if angle_deg < 35.0 {
                    self.exit_70.as_ref()
                } else if angle_deg < 50.0 {
                    self.exit_45.as_ref()
                } else {
                    // the client table has no top-exit clip for ladders 50 degrees or steeper
                    None
                }
            }
            4 => {
                if angle_deg < 1.0 {
                    self.exit_ladder.as_ref()
                } else if angle_deg < 20.0 {
                    self.exit_ladder_80.as_ref()
                } else {
                    None
                }
            }
            2 | 5 => self.exit_rope.as_ref().or(self.exit_ladder.as_ref()),
            8 => self.guardrail.up.as_ref(),
            _ => None,
        }
    }

    /// Metres per second the climb clip moves the root.
    pub fn speed(&self, kind: u8) -> f32 {
        self.family(kind).speed
    }
}

#[derive(Clone)]
struct FamilyClips {
    up: Option<aa_anim::pose::BoundClip>,
    down: Option<aa_anim::pose::BoundClip>,
    idle: Option<aa_anim::pose::BoundClip>,
    speed: f32,
}

impl FamilyClips {
    fn load(
        rig: &aa_anim::pose::Rig,
        cal: &aa_anim::Cal,
        clips: &aa_anim::ClipLibrary,
        folder: &str,
        family: Family,
    ) -> FamilyClips {
        let bind = |name: &str| bind_path(rig, cal, clips, folder, name);
        let (up, down, idle) = match family {
            // the rope and the wood wall have one clip only (their states' `ManualUpdateIdle`);
            // a stopped climber keeps it, the down pass plays it backwards
            Family::Rope => {
                let up = bind("fist_mo_rope_up");
                (up.clone(), up.clone(), up)
            }
            Family::Wood => {
                let up = bind("fist_mo_wood_up");
                (up.clone(), up.clone(), up)
            }
            // the horizontal rope: `hang_rope_front` idles, `hrope_front/back` move
            Family::Hrope => (
                bind("fist_mo_rope_front_clip"),
                bind("fist_mo_rope_back_clip"),
                bind("fist_mo_rope_clip_idle").or_else(|| bind("fist_mo_rope_front")),
            ),
            _ => (
                bind(&format!("fist_mo_{}_up", family.prefix())),
                bind(&format!("fist_mo_{}_down", family.prefix())),
                bind(&format!("fist_mo_{}_idle", family.prefix())),
            ),
        };
        let speed = up.as_ref().map_or(0.0, |c| clip_speed(rig, c));
        FamilyClips {
            up,
            down,
            idle,
            speed,
        }
    }
}

fn bind_path(
    rig: &aa_anim::pose::Rig,
    cal: &aa_anim::Cal,
    clips: &aa_anim::ClipLibrary,
    folder: &str,
    name: &str,
) -> Option<aa_anim::pose::BoundClip> {
    // the cal (with its includes) names the climb clips too, and an alias can point at another
    // file (`fist_mo_wood_up` is the ladder up clip); only a name the cal lacks uses the folder
    let path = cal
        .animation(name)
        .map(|entry| entry.path.clone())
        .unwrap_or_else(|| format!("{folder}/{name}.caf"));
    let clip = clips
        .bound(rig, &path, aa_anim::pose::SampleOptions::default())
        .ok()?;
    super::anim::remember_file(&clip, &path);
    Some(clip)
}

/// Metres per second the root bone travels in a clip (3D: the climb clips move it up).
fn clip_speed(rig: &aa_anim::pose::Rig, clip: &aa_anim::pose::BoundClip) -> f32 {
    let root = rig.skeleton.bones.get(rig.root).map(|b| b.controller_id);
    let Some(track) = root.and_then(|c| clip.clip.track(c)) else {
        return 0.0;
    };
    let (Some(a), Some(b)) = (track.sample_pos(0.0), track.sample_pos(clip.clip.duration)) else {
        return 0.0;
    };
    let travelled = (b - a).length() * clip.options.position_scale;
    if clip.clip.duration > 0.0 {
        travelled / clip.clip.duration
    } else {
        0.0
    }
}

/// The closest point on a segment to `p`: `(distance, fraction 0..1, point)`.
pub fn segment_point(p: Vec3, a: Vec3, b: Vec3) -> (f32, f32, Vec3) {
    let d = b - a;
    let len2 = d.length_squared();
    let t = if len2 > 1e-9 {
        ((p - a).dot(d) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = a + d * t;
    (p.distance(q), t, q)
}

/// The world frame of a ladder helper under a doodad root: `(bottom, top, axis, normal)`.
fn world_frame(
    helper: &LadderHelper,
    normal: Vec3,
    root: &GlobalTransform,
) -> (Vec3, Vec3, Vec3, Vec3) {
    let bottom = bevy_to_cry(root.transform_point(helper.bottom));
    let top = bevy_to_cry(root.transform_point(helper.top));
    let axis = (top - bottom).normalize_or_zero();
    let normal = bevy_to_cry(root.transform_vector(normal)).normalize_or_zero();
    (bottom, top, axis, normal)
}

/// The heading that faces the ladder from the outward normal (`normal` points from the ladder to
/// the body), in game radians (0 north, positive counter-clockwise).
pub fn facing_of_normal(normal: Vec3) -> f32 {
    let n = Vec2::new(normal.x, normal.y);
    if n.length_squared() < 1e-8 {
        return 0.0;
    }
    let n = n.normalize();
    n.x.atan2(-n.y)
}

/// The relative angle stored in the climb data for kinds 2, 3, 6, 7 and 10: the angle from the
/// helper's forward to the climber's facing, in `-pi..pi`. Kinds 1, 4, 5, 8 and 9 use the identity
/// rotation and the writer stores 127; return 0 for them.
pub fn relative_angle(kind: u8, facing: f32, helper_forward: Vec3) -> f32 {
    if matches!(kind, 1 | 4 | 5 | 8 | 9) {
        return 0.0;
    }
    let f = super::player::forward(facing);
    let h = Vec2::new(helper_forward.x, helper_forward.y).normalize_or_zero();
    if h == Vec2::ZERO {
        return 0.0;
    }
    (f.y * h.x - f.x * h.y).atan2(f.dot(h))
}

/// The parent hull pose in game coordinates, drawn pose first.
fn parent_pose(
    parent: u32,
    views: Option<&Views>,
    hull_poses: Option<&DeckHullPoses>,
    units: &Units,
) -> Option<(Vec3, Quat)> {
    if let Some(pose) = hull_poses.and_then(|h| h.0.get(&parent)).copied() {
        return Some((pose.0, pose.1));
    }
    if let Some(pose) = views.and_then(|v| v.unit_hull_pose(parent)) {
        return Some((pose.0, pose.1));
    }
    units.by_id.get(&parent).map(|u| {
        (
            Vec3::new(u.pos.x, u.pos.y, u.pos.z),
            Quat::from_rotation_z(u.yaw()),
        )
    })
}

/// The local body point for `fraction` along the ladder and the outward normal.
fn body_point(bottom: Vec3, axis: Vec3, normal: Vec3, length: f32, fraction: f32) -> Vec3 {
    bottom + axis * (fraction.clamp(0.0, 1.0) * length) + normal * STANDOFF
}

/// This frame's global transform of `entity`: the local `Transform`s of the `ChildOf` chain
/// composed now. `GlobalTransform` still holds last frame's pose in `Update`, so a doodad on a
/// moving hull would otherwise lag a frame behind the body and the packet.
fn current_global(
    entity: Entity,
    transforms: &Query<(&Transform, Option<&ChildOf>), Without<LocalPlayer>>,
) -> Option<GlobalTransform> {
    let (transform, mut parent) = transforms.get(entity).ok()?;
    let mut global = GlobalTransform::from(*transform);
    let mut guard = 0;
    while let Some(child_of) = parent {
        let Ok((parent_transform, next)) = transforms.get(child_of.parent()) else {
            break;
        };
        global = GlobalTransform::from(*parent_transform) * global;
        parent = next;
        guard += 1;
        if guard > 64 {
            break;
        }
    }
    Some(global)
}

/// The climb doodad under the use trace: the nearest `$ladder` helper within [`REACH`].
struct LadderHit {
    obj_id: u32,
    kind: u8,
    helper: LadderHelper,
    parent: u32,
    attach_point: u8,
    root_rotation: Quat,
    bottom: Vec3,
    top: Vec3,
    axis: Vec3,
    closest: Vec3,
    length: f32,
}

fn nearest_ladder(
    ladders: &Query<(&RemoteDoodad, &DoodadVisual, Entity)>,
    transforms: &Query<(&Transform, Option<&ChildOf>), Without<LocalPlayer>>,
    doodads: &Doodads,
    db: Option<&Db>,
    pos: Vec3,
) -> Option<LadderHit> {
    let db = db?;
    let mut best: Option<(f32, LadderHit)> = None;
    for (doodad, visual, entity) in ladders {
        let Some(helper) = visual.ladder else {
            continue;
        };
        let Some(source) = doodads.by_id.get(&doodad.obj_id) else {
            continue;
        };
        let Some(kind) = db.ladder_kind(visual.func_group_id) else {
            continue;
        };
        let Some(root) = current_global(entity, transforms) else {
            continue;
        };
        let bottom = bevy_to_cry(root.transform_point(helper.bottom));
        let top = bevy_to_cry(root.transform_point(helper.top));
        let length = bottom.distance(top);
        if length < 0.2 {
            continue;
        }
        let (dist, _, closest) = segment_point(pos, bottom, top);
        if dist > REACH {
            continue;
        }
        if best.as_ref().is_none_or(|b| dist < b.0) {
            best = Some((
                dist,
                LadderHit {
                    obj_id: doodad.obj_id,
                    kind,
                    helper,
                    parent: source.parent_obj_id,
                    attach_point: source.attach_point,
                    root_rotation: root.rotation(),
                    bottom,
                    top,
                    axis: (top - bottom).normalize_or_zero(),
                    closest,
                    length,
                },
            ));
        }
    }
    best.map(|(_, hit)| hit)
}

/// What the bottom exit does at a point.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Rest {
    /// Stand on the ground at this height.
    Stand(f32),
    Swim {
        surface: f32,
        kind: WaterKind,
    },
    /// Neither ground nor deep water is close: fall.
    Fall,
}

fn bottom_rest(at: Vec3, ground: Option<f32>, wet: Option<Wet>, enter: f32) -> Rest {
    if let Some(w) = wet.filter(|w| super::swim::should_swim(false, at.z, Some(*w), enter)) {
        return Rest::Swim {
            surface: w.surface,
            kind: w.kind,
        };
    }
    match ground {
        Some(g) if at.z <= g || at.z - g <= BOTTOM_SNAP => Rest::Stand(g),
        _ => Rest::Fall,
    }
}

/// The bottom exit: stand on the ground (the terrain or a brush such as a dock or a lower deck,
/// through the same support query movement uses), swim when the water is deep enough, or fall when
/// neither is close. Never reports `grounded` at a point above the ground or in water.
fn land_after_climb(
    mover: &mut Mover,
    terrain: Option<&TerrainQuery>,
    collision: Option<&WorldCollision>,
    water: Option<&WaterQuery>,
    actor: ActorParams,
) {
    let at = mover.pos;
    let ground = match (terrain, collision) {
        (Some(terrain), Some(collision)) => {
            player::support_at(terrain, collision, at, player::STEP_UP)
        }
        (Some(terrain), None) => terrain.ground_at(at.x, at.y).map(|g| g.height),
        _ => None,
    };
    let wet = water
        .and_then(|w| w.water_at(at))
        .map(|(surface, kind)| Wet {
            surface,
            kind,
            depth: (surface - ground.unwrap_or(at.z)).max(0.0),
        });
    match bottom_rest(at, ground, wet, actor.swim_depth()) {
        Rest::Stand(g) => {
            mover.pos.z = g;
            mover.grounded = true;
            mover.vz = 0.0;
        }
        Rest::Swim { surface, kind } => {
            mover.swim = Some(Swim {
                surface,
                kind,
                local_dir: Vec2::ZERO,
                speed: 0.0,
                vertical: 0.0,
                deep: false,
            });
            mover.grounded = false;
            mover.vz = 0.0;
        }
        Rest::Fall => {
            mover.grounded = false;
            mover.vz = 0.0;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn control_ladder(
    mut commands: Commands,
    time: Res<Time>,
    actions: Res<crate::settings::Actions>,
    typing: Option<Res<crate::hud::HudKeyboardFocus>>,
    library: Res<aa_anim::ClipLibrary>,
    mut player: Query<(
        Entity,
        &LocalPlayer,
        &mut Mover,
        &mut Transform,
        &mut Motion,
        Option<&Ladder>,
        Option<&Animator>,
        Option<&Locomotion>,
        Has<super::combat::Dead>,
        Option<&super::skill_motion::SkillMove>,
        Has<super::seating::Seated>,
        Has<super::mount::Rider>,
        Option<&ActorParams>,
    )>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    ladders: Query<(&RemoteDoodad, &DoodadVisual, Entity)>,
    transforms: Query<(&Transform, Option<&ChildOf>), Without<LocalPlayer>>,
    mut world: LadderWorld,
    mut out: MessageWriter<NetCommand>,
    mut state: ResMut<SendState>,
    mut input_state: ResMut<PlayerInput>,
) {
    let Ok((
        entity,
        _local,
        mut mover,
        mut transform,
        mut motion,
        ladder,
        animator,
        locomotion,
        dead,
        skill_moving,
        seated,
        rider,
        actor,
    )) = player.single_mut()
    else {
        // no local player: nothing may keep F claimed
        world.claim.0 = None;
        return;
    };
    let typing = typing.is_some_and(|f| f.0);
    let dt = time.delta_secs().min(0.1);
    let used = !typing && actions.just_pressed("do_interaction_1", 0);
    let jump = !typing && actions.just_pressed("jump", 0);
    let up = if typing {
        0.0
    } else {
        actions.pressed("moveforward", 0) as i32 as f32
            - actions.pressed("moveback", 0) as i32 as f32
    };

    if dead {
        world.claim.0 = None;
        if ladder.is_some() {
            // The native value-change callback (`FUN_39137320`) leaves a hanging local unit with
            // 0xb when its value reaches 0, and sends only the unhang (no release movement).
            commands.entity(entity).remove::<Ladder>();
            motion.climb = None;
            state.after_ladder();
            out.write(NetCommand(Command::Unhang {
                reason: reason::LADDER_LOST,
            }));
        }
        return;
    }

    let Some(ladder) = ladder else {
        // a teleport or a mount took the ladder away without `release`: drop the climb pose
        motion.climb = None;
        let found = nearest_ladder(
            &ladders,
            &transforms,
            &doodads,
            world.db.as_deref(),
            mover.pos,
        );
        // the native grab gate 1.4(b) (`FUN_393b8f40`): a unit BOUND to a vehicle (`+0x3af0`,
        // its seat at `+0x3af4`), on a seat, in a skill move or flying does not hang; a swimmer
        // does (there is no swim gate in `FUN_393630e0`). A deck walker sends its own moves with
        // `+0x3af0` null (`FUN_394045f0`), so a deck is not a vehicle binding.
        let blocked = world.mount.as_ref().is_some_and(|m| m.0.is_some())
            || rider
            || seated
            || skill_moving.is_some()
            || world.gliding.as_ref().is_some_and(|g| g.flying());
        // F belongs to the ladder only when the grab would be accepted (a refused grab must not
        // take talk or loot).
        world.claim.0 = if blocked {
            None
        } else {
            found.as_ref().map(|hit| hit.obj_id)
        };
        if !used || blocked {
            return;
        }
        let Some(LadderHit {
            obj_id,
            kind,
            helper,
            parent,
            attach_point,
            root_rotation,
            bottom,
            top: _,
            axis,
            closest,
            length,
        }) = found
        else {
            return;
        };
        let helper_forward_game = bevy_to_cry(root_rotation * helper.forward).normalize_or_zero();
        let mut normal = Vec3::new(mover.pos.x - closest.x, mover.pos.y - closest.y, 0.0);
        if normal.length_squared() < 1e-6 {
            normal = helper_forward_game;
            normal.z = 0.0;
        }
        normal = normal.normalize_or_zero();
        if normal == Vec3::ZERO {
            // Straight below a vertical ladder: face its helper's side.
            normal = helper_forward_game;
            normal.z = 0.0;
            normal = normal.normalize_or_zero();
        }
        let facing = facing_of_normal(normal);
        let x_m = closest.distance(bottom).max(0.05).min(length);
        let fraction = (x_m / length).clamp(0.0, 1.0);
        let angle = relative_angle(kind, facing, helper_forward_game);
        let climb_data = climb::pack(x_m, 0.0, angle);
        // The outward normal back in the doodad root's Bevy space.
        let normal_bevy = aa_world::cry_to_bevy(normal);
        let normal_local = root_rotation
            .inverse()
            .mul_vec3(normal_bevy)
            .normalize_or_zero();
        let speed = locomotive_speed(
            kind,
            animator,
            locomotion,
            &library,
            world.sprint.as_deref(),
        );
        let ladder = Ladder {
            obj_id,
            kind,
            parent,
            attach_point,
            helper,
            normal: normal_local,
            fraction,
            climb_data,
            angle_deg: axis.z.clamp(-1.0, 1.0).acos().to_degrees(),
            speed,
            exiting: None,
        };
        let world_point = body_point(bottom, axis, normal, length, fraction);
        mover.pos = world_point;
        mover.facing = facing;
        mover.vz = 0.0;
        mover.grounded = false;
        mover.velocity = Vec3::ZERO;
        mover.swim = None;
        *transform = player::placement(world_point, facing, 1.0);
        commands
            .entity(entity)
            .remove::<super::ship::DeckRide>()
            .remove::<player::CarriedVelocity>();
        commands.entity(entity).insert(ladder.clone());
        motion.climb = Some(ClimbPose {
            kind,
            up: 0.0,
            exiting: false,
            angle_deg: ladder.angle_deg,
        });
        *input_state = PlayerInput::default();
        out.write(NetCommand(Command::Hang {
            target_obj_id: obj_id,
        }));
        state.after_ladder();
        send_climb(
            &mut out,
            &mover,
            &ladder,
            &units,
            world.views.as_deref(),
            world.hull_poses.as_deref(),
            0.0,
            false,
        );
        state.ladder_sent(false);
        info!("ladder: grabbed doodad {obj_id} kind {kind} at {fraction:.2} of {length:.2} m");
        return;
    };
    world.claim.0 = Some(ladder.obj_id);

    let mut ladder = ladder.clone();
    // No posture change releases the ladder: the only native 0xb site is the death check above.
    // Mounting or a moving skill while already hanging keeps the hang (the grab gate keeps a
    // mounted or skill-moving unit from grabbing in the first place).
    if let Some(exit) = ladder.exiting {
        let left = exit.left - dt;
        if left > 0.0 {
            ladder.exiting = Some(Exit { left, ..exit });
            motion.climb = Some(ClimbPose {
                kind: ladder.kind,
                up: 0.0,
                exiting: true,
                angle_deg: ladder.angle_deg,
            });
            *input_state = PlayerInput::default();
            commands.entity(entity).insert(ladder);
            return;
        }
        finish_exit(
            &mut commands,
            entity,
            &mut mover,
            &mut motion,
            &mut transform,
            &mut state,
            &mut out,
            exit.top,
            ladder.kind,
        );
        return;
    }

    // Find the drawn doodad again; a vanished ladder is `FUN_39137470`'s "my ladder is missing".
    let Some((_, _, root_entity)) = ladders.iter().find(|(d, _, _)| d.obj_id == ladder.obj_id)
    else {
        release(
            &mut commands,
            entity,
            &mut mover,
            &mut motion,
            &mut state,
            &mut out,
            reason::LADDER_MISSING,
        );
        return;
    };
    let Some(root) = current_global(root_entity, &transforms) else {
        release(
            &mut commands,
            entity,
            &mut mover,
            &mut motion,
            &mut state,
            &mut out,
            reason::LADDER_MISSING,
        );
        return;
    };
    let (bottom, top, axis, normal) = world_frame(&ladder.helper, ladder.normal, &root);
    let length = bottom.distance(top);
    if length < 0.2 {
        release(
            &mut commands,
            entity,
            &mut mover,
            &mut motion,
            &mut state,
            &mut out,
            reason::LADDER_MISSING,
        );
        return;
    }
    ladder.angle_deg = axis.z.clamp(-1.0, 1.0).acos().to_degrees();
    let facing = facing_of_normal(normal);

    if jump {
        let forward = player::forward(facing);
        mover.vz = JUMP_UP;
        mover.grounded = false;
        commands
            .entity(entity)
            .insert(player::CarriedVelocity(forward * JUMP_FORWARD));
        release(
            &mut commands,
            entity,
            &mut mover,
            &mut motion,
            &mut state,
            &mut out,
            reason::FORWARD_JUMP,
        );
        return;
    }
    if used {
        release(
            &mut commands,
            entity,
            &mut mover,
            &mut motion,
            &mut state,
            &mut out,
            reason::RELEASE,
        );
        return;
    }

    let speed = ladder.speed * world.sprint.as_ref().map_or(1.0, |s| s.ladder_speed_mul);
    ladder.fraction = (ladder.fraction + up * speed * dt / length).clamp(0.0, 1.0);
    let moving = up != 0.0;
    mover.pos = body_point(bottom, axis, normal, length, ladder.fraction);
    mover.facing = facing;
    mover.vz = up * speed;
    mover.grounded = false;
    mover.velocity = Vec3::new(0.0, 0.0, mover.vz);
    mover.swim = None;
    *transform = player::placement(mover.pos, facing, 1.0);
    motion.climb = Some(ClimbPose {
        kind: ladder.kind,
        up: up * speed,
        exiting: false,
        angle_deg: ladder.angle_deg,
    });
    *input_state = PlayerInput {
        moving,
        jumped: false,
        turned: false,
    };

    if up > 0.0 && ladder.fraction >= 1.0 - EPS {
        ladder.exiting = Some(Exit {
            left: TOP_EXIT_SECS,
            top: true,
        });
        commands.entity(entity).insert(ladder);
        state.after_ladder();
        return;
    }
    if up < 0.0 && ladder.fraction <= EPS {
        mover.pos = body_point(bottom, axis, normal, length, 0.0);
        land_after_climb(
            &mut mover,
            world.terrain.as_deref(),
            world.collision.as_deref(),
            world.water.as_deref(),
            actor.copied().unwrap_or_default(),
        );
        *transform = player::placement(mover.pos, facing, 1.0);
        release(
            &mut commands,
            entity,
            &mut mover,
            &mut motion,
            &mut state,
            &mut out,
            reason::RELEASE,
        );
        return;
    }
    commands.entity(entity).insert(ladder.clone());
    state.ladder_clock(dt);
    if state.ladder_due(moving) {
        send_climb(
            &mut out,
            &mover,
            &ladder,
            &units,
            world.views.as_deref(),
            world.hull_poses.as_deref(),
            up * speed,
            moving,
        );
        state.ladder_sent(moving);
    }
}

/// The clip's root speed for `kind`, or the fallback when the model has no climb clips.
fn locomotive_speed(
    kind: u8,
    animator: Option<&Animator>,
    loco: Option<&Locomotion>,
    library: &aa_anim::ClipLibrary,
    sprint: Option<&super::sprint::Sprint>,
) -> f32 {
    let speed = animator
        .zip(loco)
        .and_then(|(a, l)| l.clips.climb_clips(&a.rig, library).map(|c| c.speed(kind)))
        .filter(|s| *s > 0.1)
        .unwrap_or(FALLBACK_CLIMB_SPEED);
    speed * sprint.map_or(1.0, |s| s.ladder_speed_mul)
}

/// Writes the hanging movement. A parented ladder (a ship's side ladder) also gets the `0x20`
/// block: the primary position is hull-local and the auxiliary frame is the world point.
#[allow(clippy::too_many_arguments)]
fn send_climb(
    out: &mut MessageWriter<NetCommand>,
    mover: &Mover,
    ladder: &Ladder,
    units: &Units,
    views: Option<&Views>,
    hull_poses: Option<&DeckHullPoses>,
    up: f32,
    moving: bool,
) {
    let world = mover.pos;
    let mut primary = Pos::new(world.x, world.y, world.z);
    let mut yaw = mover.facing;
    let mut support = MoveSupport {
        obj_id: 1,
        climb: Some(ladder.climb_data),
        ..MoveSupport::default()
    };
    if ladder.parent != 0
        && let Some((root, rotation)) = parent_pose(ladder.parent, views, hull_poses, units)
    {
        let local = super::ship::deck::world_to_ship(root, rotation, world);
        primary = Pos::new(local.x, local.y, local.z);
        // The primary position is hull-local, so its direction byte is the facing in the hull
        // frame; the world facing stays in the auxiliary block, as `deck_move` sends it.
        yaw = super::cart::wrap_angle(mover.facing - super::ship::deck::yaw_of(rotation));
        support = MoveSupport {
            obj_id: ladder.parent,
            kind: 1,
            part: u16::from(ladder.attach_point),
            world: Some((
                Pos::new(world.x, world.y, world.z),
                [0, 0, radians_to_direction(mover.facing)],
            )),
            climb: Some(ladder.climb_data),
        };
    }
    let to_wire = |v: f32| (v * 1024.0).round().clamp(-32767.0, 32767.0) as i16;
    let cmd = MoveCommand {
        pos: primary,
        yaw,
        moving,
        stance: stance::CROUCH,
        velocity: [0, 0, to_wire(up)],
        delta_movement: [0, 0, (up.clamp(-1.0, 1.0) * 127.0) as i8],
        state: MoveState::Hang,
        support: Some(support),
        ..MoveCommand::default()
    };
    out.write(NetCommand(Command::Move(cmd)));
}

/// Leaves the ladder: removes the component, sends `CSUnhang` and one release movement without
/// the hanging bit (the server detaches its sticky parent and any hull parent).
fn release(
    commands: &mut Commands,
    entity: Entity,
    mover: &mut Mover,
    motion: &mut Motion,
    state: &mut SendState,
    out: &mut MessageWriter<NetCommand>,
    reason: i32,
) {
    commands.entity(entity).remove::<Ladder>();
    motion.climb = None;
    state.after_ladder();
    out.write(NetCommand(Command::Unhang { reason }));
    let to_wire = |v: f32| (v * 1024.0).round().clamp(-32767.0, 32767.0) as i16;
    let cmd = MoveCommand {
        pos: Pos::new(mover.pos.x, mover.pos.y, mover.pos.z),
        yaw: mover.facing,
        moving: false,
        stance: stance::RELAXED,
        velocity: [0, 0, to_wire(mover.vz)],
        delta_movement: [0; 3],
        state: if mover.grounded {
            MoveState::Ground
        } else {
            MoveState::Fall
        },
        support: None,
        ..MoveCommand::default()
    };
    out.write(NetCommand(Command::Move(cmd)));
}

/// The kinds whose state table has no top-exit clip: the wood wall (3), the mast (7) and the big
/// tree (10) (`LadderClips::exit` returns `None` for them), so reaching their top leaves the
/// ladder in place instead of stepping over it. A horizontal rope (5) also ends at its point.
fn steps_over_top(kind: u8) -> bool {
    !matches!(kind, 3 | 5 | 7 | 10)
}

/// The top exit finished: step over the ladder onto the platform, or stand at the bottom.
fn finish_exit(
    commands: &mut Commands,
    entity: Entity,
    mover: &mut Mover,
    motion: &mut Motion,
    transform: &mut Transform,
    state: &mut SendState,
    out: &mut MessageWriter<NetCommand>,
    top: bool,
    kind: u8,
) {
    if top {
        if steps_over_top(kind) {
            let f = player::forward(mover.facing);
            mover.pos += Vec3::new(f.x, f.y, 0.0) * TOP_STEP;
            mover.pos.z += 0.05;
        }
        mover.grounded = false;
        mover.vz = 0.0;
    } else {
        mover.grounded = false;
        mover.vz = 0.0;
    }
    *transform = player::placement(mover.pos, mover.facing, 1.0);
    release(commands, entity, mover, motion, state, out, reason::RELEASE);
}

/// The climb animation: the up/down/idle clip of the kind, or the top exit clip while it runs.
/// Leaving the ladder makes locomotion pick its clip again.
pub fn drive_climb(
    library: Res<aa_anim::ClipLibrary>,
    mut player: Query<(
        &Motion,
        &mut Animator,
        &mut Locomotion,
        Option<&super::combat::Acting>,
        Has<super::combat::Dead>,
    )>,
) {
    for (motion, mut animator, mut loco, acting, dead) in &mut player {
        let Some(climb) = motion.climb else {
            if loco.on_climb {
                loco.on_climb = false;
                loco.invalidate();
            }
            continue;
        };
        if dead || acting.is_some_and(|a| !a.layered) {
            continue;
        }
        let Some(clips) = loco.clips.climb_clips(&animator.rig, &library) else {
            continue;
        };
        let clip = if climb.exiting {
            clips.exit(climb.kind, climb.angle_deg)
        } else if climb.up > 0.05 {
            clips.up(climb.kind)
        } else if climb.up < -0.05 {
            clips.down(climb.kind)
        } else {
            clips.idle(climb.kind)
        };
        let Some(clip) = clip else { continue };
        let playing = animator
            .current
            .as_ref()
            .is_some_and(|p| std::sync::Arc::ptr_eq(&p.clip.clip, &clip.clip));
        if !playing {
            // an exit plays once and holds its last frame; climbing loops
            animator.play(clip.clone(), !climb.exiting, 0.2);
            animator.random_idle = None;
        }
        loco.on_climb = true;
        animator.speed = if climb.exiting {
            1.0
        } else {
            let base = clips.speed(climb.kind);
            if base > 0.1 && climb.up.abs() > 0.05 {
                (climb.up.abs() / base).clamp(0.4, 2.0)
            } else {
                1.0
            }
        };
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<LadderClaim>()
        .add_systems(
            Update,
            control_ladder
                .after(super::ship::deck_control)
                // `DeckHullPoses` is this frame's hull pose for the hull-local packet point
                .after(super::units::place_deck_riders)
                .before(super::player::control_player)
                // the HUD's doodad/loot key claim reads this frame's ladder reach
                .before(crate::hud::doodad::TrackDoodads)
                .run_if(super::world_screens),
        )
        .add_systems(
            Update,
            drive_climb
                .after(super::anim::drive_locomotion)
                .run_if(super::world_screens),
        );
}

#[cfg(test)]
mod tests;
