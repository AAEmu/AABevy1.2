//! Helper placement and the CAL-resolved action19 base sequence.

use aa_anim::{
    Animator, Cal, Character, ClipLibrary,
    pose::{BoundClip, Rig},
};
use aa_net::state::{Doodads, Units};
use aa_world::coords::{bevy_to_cry, cry_matrix_to_bevy};
use bevy::math::Affine3A;
use bevy::prelude::*;
use std::sync::Arc;

use super::{SeatReceipt, Seats};
use crate::ingame::{
    anim::{Locomotion, Motion, bind_clip},
    combat::{Acting, Dead},
    doodad_visual::DoodadVisual,
    player::{LocalPlayer, Mover},
    units::{RemoteDoodad, RemoteUnit, Views},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Source {
    owner: Entity,
    generation: u64,
    model: Entity,
    phase: u32,
}

/// The actor remains independently owned: destroying a doodad cannot despawn it.
#[derive(Component)]
pub(crate) struct Seated {
    receipt: SeatReceipt,
    source: Source,
    relative: Affine3A,
    last: Transform,
}

#[cfg(test)]
impl Seated {
    /// A settled seat for the tests that only look at the marker.
    pub(crate) fn fixture() -> Seated {
        Seated {
            receipt: SeatReceipt {
                generation: 0,
                receipt: 0,
                bond: aa_net::DoodadBond {
                    owner_obj_id: 0,
                    attach_point: 0,
                    kind: 0,
                    space: 0,
                    spot: 0,
                },
            },
            source: Source {
                owner: Entity::PLACEHOLDER,
                generation: 0,
                model: Entity::PLACEHOLDER,
                phase: 0,
            },
            relative: Affine3A::IDENTITY,
            last: Transform::IDENTITY,
        }
    }
}

struct Clips {
    start: BoundClip,
    cycle: BoundClip,
    end: BoundClip,
}

impl Clips {
    fn bind(character: &Character, animator: &Animator, library: &ClipLibrary) -> Option<Self> {
        let cal = Cal::shared(&character.assets.cal).ok()?;
        let bind = |name| bind_clip(&animator.rig, &cal, library, name);
        Some(Self {
            start: bind("fist_ac_sit_down_start")?,
            cycle: bind("fist_ac_sit_down_loop")?,
            end: bind("fist_ac_sit_up")?,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pose {
    Start,
    Interrupted,
    End,
}

/// Retained while a performance owns the animator, and until sit-up has ended.
#[derive(Component)]
pub(crate) struct SeatPose {
    clips: Option<Clips>,
    playing: Pose,
    rig: Arc<Rig>,
    cal: String,
}

fn initial(parent: Affine3A, helper: Option<Affine3A>) -> Option<(Transform, Affine3A)> {
    let world = parent * helper.unwrap_or(Affine3A::IDENTITY);
    if !world.is_finite() || !parent.is_finite() || parent.matrix3.determinant().abs() < 1e-12 {
        return None;
    }
    let (_, rotation, translation) = world.to_scale_rotation_translation();
    if !rotation.is_finite() || !translation.is_finite() {
        return None;
    }
    let placed = Transform {
        translation,
        rotation: rotation.normalize(),
        scale: Vec3::ONE,
    };
    Some((placed, parent.inverse() * placed.compute_affine()))
}

fn sync_position(
    transform: &Transform,
    mover: Option<&mut Mover>,
    remote: Option<&mut RemoteUnit>,
    detached: bool,
) {
    let pos = bevy_to_cry(transform.translation);
    let forward = transform.rotation * Vec3::NEG_Z;
    let facing = (-forward.x).atan2(-forward.z);
    if let Some(mover) = mover {
        mover.pos = pos;
        mover.facing = facing;
        mover.velocity = Vec3::ZERO;
        mover.vz = 0.0;
        mover.grounded = true;
        mover.swim = None;
    }
    if let Some(remote) = remote {
        remote.pos = pos;
        remote.facing = facing;
        if detached {
            remote.reset_track(pos, facing);
        }
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn follow(
    mut commands: Commands,
    mut seats: ResMut<Seats>,
    views: Res<Views>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    player: Option<Res<aa_net::state::Player>>,
    mount: Option<Res<crate::ingame::LocalMount>>,
    parents: Query<(&DoodadVisual, &Transform, Option<&ChildOf>), With<RemoteDoodad>>,
    globals: Query<&GlobalTransform>,
    mut actors: Query<
        (
            Entity,
            Option<&LocalPlayer>,
            Option<&mut RemoteUnit>,
            Option<&mut Mover>,
            &mut Transform,
            Option<&mut Motion>,
            Option<&mut Seated>,
            Option<&mut Locomotion>,
            Has<Dead>,
        ),
        (
            Without<RemoteDoodad>,
            Or<(With<LocalPlayer>, With<RemoteUnit>)>,
        ),
    >,
) {
    for (entity, local, mut remote, mut mover, mut transform, motion, attached, loco, dead) in
        &mut actors
    {
        let Some(obj_id) = local
            .map(|l| l.obj_id)
            .or_else(|| remote.as_ref().map(|r| r.obj_id))
        else {
            continue;
        };
        let released_report = seats.release_has_report(obj_id);
        let mounted = if local.is_some() {
            mount.as_ref().is_some_and(|m| m.0.is_some())
                || player
                    .as_ref()
                    .and_then(|p| p.spawn.as_ref())
                    .is_some_and(|p| p.unit.attached_to.is_some())
        } else {
            units
                .by_id
                .get(&obj_id)
                .is_some_and(|u| u.attached_to.is_some())
        };
        let wanted = seats
            .wanted(obj_id)
            .filter(|r| r.bond.kind == 2 && !mounted && !dead);
        let ready = wanted.and_then(|receipt| {
            let owner = receipt.bond.owner_obj_id;
            let parent = views.doodad_entity(owner)?;
            let (visual, parent_transform, child_of) = parents.get(parent).ok()?;
            let phase = doodads.by_id.get(&owner)?.func_group_id;
            if visual.func_group_id != phase || visual.rendered_func_group_id != phase {
                return None;
            }
            let source = Source {
                owner: parent,
                generation: views.doodad_generation(owner)?,
                model: visual.model?,
                phase,
            };
            let existing = attached.as_ref().is_some_and(|old| {
                old.receipt.generation == receipt.generation
                    && old.receipt.bond == receipt.bond
                    && old.source == source
            });
            let helper = if existing {
                None
            } else {
                visual
                    .bond_helpers
                    .get(&format!("bond{}", receipt.bond.spot))
                    .map(|m| Affine3A::from_mat4(cry_matrix_to_bevy(Mat4::from(*m))))
            };
            // a doodad attached to a unit (a vehicle, a house) has a Transform local to that unit's root
            let world = match child_of {
                Some(child_of) => globals.get(child_of.parent()).ok()?.affine() * parent_transform.compute_affine(),
                None => parent_transform.compute_affine(),
            };
            Some((receipt, source, world, helper))
        });

        let mut attached = attached;
        // A changed accepted receipt may be identical. Keep its physical handoff when
        // the same actor/source/bond is already attached; only update completion identity.
        let same =
            attached
                .as_ref()
                .zip(ready.as_ref())
                .is_some_and(|(old, (receipt, source, ..))| {
                    old.receipt.generation == receipt.generation
                        && old.receipt.bond == receipt.bond
                        && old.source == *source
                });
        if !same && let Some(old) = attached.take() {
            *transform = old.last;
            sync_position(
                &transform,
                mover.as_deref_mut(),
                remote.as_deref_mut(),
                !released_report,
            );
            commands.entity(entity).remove::<Seated>();
            if let Some(mut loco) = loco {
                loco.invalidate();
            }
        }
        if let Some((receipt, source, parent, helper)) = ready {
            if same {
                let mut old = attached.unwrap();
                old.receipt = receipt;
                let followed = Transform::from_matrix(Mat4::from(parent * old.relative));
                if followed != *transform {
                    *transform = followed;
                }
                old.last = *transform;
            } else if let Some((placed, relative)) = initial(parent, helper) {
                *transform = placed;
                commands.entity(entity).insert(Seated {
                    receipt,
                    source,
                    relative,
                    last: placed,
                });
            } else {
                continue;
            }
            sync_position(
                &transform,
                mover.as_deref_mut(),
                remote.as_deref_mut(),
                false,
            );
            if let Some(mut motion) = motion {
                *motion = Motion::default();
            }
        }
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn pose(
    mut commands: Commands,
    library: Res<ClipLibrary>,
    time: Res<Time>,
    mut actors: Query<(
        Entity,
        &Character,
        &mut Animator,
        Option<&Seated>,
        Option<&mut SeatPose>,
        Option<&Acting>,
        Option<&mut Locomotion>,
        Has<Dead>,
    )>,
) {
    for (entity, character, mut animator, attached, pose, acting, loco, dead) in &mut actors {
        if !animator.starts_enabled() { continue; }
        let acting = acting.is_some_and(|a| time.elapsed_secs_f64() < a.until);
        let Some(mut pose) = pose else {
            if attached.is_some() && !dead {
                let clips = Clips::bind(character, &animator, &library);
                if let Some(clips) = clips.as_ref().filter(|_| !acting) {
                    animator.play_once_then(clips.start.clone(), clips.cycle.clone(), 0.2);
                    animator.random_idle = None;
                    animator.speed = 1.0;
                }
                commands.entity(entity).insert(SeatPose {
                    clips,
                    playing: if acting {
                        Pose::Interrupted
                    } else {
                        Pose::Start
                    },
                    rig: animator.rig.clone(),
                    cal: character.assets.cal.clone(),
                });
            }
            continue;
        };
        if !Arc::ptr_eq(&pose.rig, &animator.rig) || pose.cal != character.assets.cal {
            pose.clips = Clips::bind(character, &animator, &library);
            pose.rig = animator.rig.clone();
            pose.cal = character.assets.cal.clone();
            pose.playing = Pose::Interrupted;
        }
        let Some(clips) = &pose.clips else {
            if attached.is_none() || dead {
                commands.entity(entity).remove::<SeatPose>();
            }
            continue;
        };
        if dead || (attached.is_none() && acting) {
            commands.entity(entity).remove::<SeatPose>();
            if let Some(mut loco) = loco {
                loco.invalidate();
            }
        } else if attached.is_none() {
            if pose.playing != Pose::End {
                animator.play(clips.end.clone(), false, 0.2);
                animator.random_idle = None;
                animator.speed = 1.0;
                pose.playing = Pose::End;
            } else if animator.current.as_ref().is_none_or(|p| p.finished()) {
                commands.entity(entity).remove::<SeatPose>();
                if let Some(mut loco) = loco {
                    loco.invalidate();
                }
            }
        } else if acting {
            pose.playing = Pose::Interrupted;
        } else if pose.playing == Pose::Interrupted {
            // Ending a real performance returns to the retained base loop, not sit-down.
            animator.play(clips.cycle.clone(), true, 0.1);
            animator.random_idle = None;
            animator.speed = 1.0;
            pose.playing = Pose::Start;
        } else if pose.playing == Pose::End {
            animator.play_once_then(clips.start.clone(), clips.cycle.clone(), 0.2);
            animator.random_idle = None;
            animator.speed = 1.0;
            pose.playing = Pose::Start;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_strips_scale_once_then_follows_the_native_parent_relative_affine() {
        let parent = Affine3A::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(1.0),
            Vec3::new(3.0, 4.0, 5.0),
        );
        let helper = Affine3A::from_translation(Vec3::new(0.0, 0.0, -1.3));
        let (placed, relative) = initial(parent, Some(helper)).unwrap();
        assert_eq!(placed.scale, Vec3::ONE);
        assert!(
            placed
                .translation
                .abs_diff_eq(parent.transform_point3(helper.translation.into()), 1e-6)
        );
        let (scale, _, _) = relative.to_scale_rotation_translation();
        assert!(scale.abs_diff_eq(Vec3::splat(0.5), 1e-6));
        let moved = Affine3A::from_scale_rotation_translation(
            Vec3::splat(3.0),
            Quat::from_rotation_y(1.2),
            Vec3::new(8.0, 4.0, 5.0),
        );
        let followed = Transform::from_matrix(Mat4::from(moved * relative));
        assert!(
            followed.scale.abs_diff_eq(Vec3::splat(1.5), 1e-5),
            "do not re-strip every frame"
        );
        assert!(
            initial(parent, None)
                .unwrap()
                .0
                .translation
                .abs_diff_eq(parent.translation.into(), 1e-6)
        );
        assert!(initial(Affine3A::from_scale(Vec3::ZERO), None).is_none());
    }
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
pub(crate) use lifecycle_tests::app as fixture_app;
