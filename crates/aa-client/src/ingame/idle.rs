//! Idle clips of other units that [`super::anim::drive_locomotion`] does not know: a creature's combat stance
//! (`fist_ba_combat_idle`, played while the monster stands in a fight) and the pose of an NPC (a merchant's or a guard's
//! stance, `anim_actions.anim_name`: `fist_pos_stn_armor_dealer_idle`, `fist_pos_sit_chair_rest_idle`, ...).
//!
//! Locomotion owns the clip while the unit moves, jumps or acts; once it stands, this system starts the combat or pose
//! clip over the plain idle, and hands the unit back to locomotion when it is not wanted any more.

use std::sync::Arc;

use aa_anim::{Animator, Cal, Character, ClipLibrary, pose::BoundClip, pose::Rig};
use aa_net::{Event, NetEvent};
use bevy::prelude::*;

use super::anim::{Locomotion, Motion, bind_clip};
use super::combat::{Acting, Dead};
use super::data::{Db, Tool};
use super::tools::PostureTool;
use super::units::{RemoteUnit, Views};
use super::weapons::Drawn;

/// The idle clips a unit has besides the relaxed one.
#[derive(Component)]
pub struct IdleClips {
    /// The pose the NPC stands in (`Unit::anim_action`), for as long as it has one.
    pub pose: Option<BoundClip>,
    /// The fighting stance of a creature.
    pub combat: Option<BoundClip>,
    /// The tools the pose holds (`anim_actions.mainhand_tool_id` / `offhand_tool_id`): the florist's flowerpot.
    pub tools: [Option<Tool>; 2],
    playing: Which,
}

impl IdleClips {
    pub fn new(pose: Option<BoundClip>, combat: Option<BoundClip>) -> IdleClips {
        Self::with_tools(pose, combat, [None, None])
    }

    pub fn with_tools(pose: Option<BoundClip>, combat: Option<BoundClip>, tools: [Option<Tool>; 2]) -> IdleClips {
        IdleClips { pose, combat, tools, playing: Which::Relaxed }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Relaxed,
    Pose,
    Combat,
}

/// The clip `anim_actions.id` names, bound to the skeleton and `.cal` of a unit; `None` for no action or when the model's
/// `.cal` does not have the clip. Callable from any thread.
pub fn pose_clip(db: &Db, anim_action: u32, skeleton: &str, cal: &str, clips: &ClipLibrary) -> Option<BoundClip> {
    if anim_action == 0 {
        return None;
    }
    let name = db.anim_action_clip(anim_action)?;
    let rig = aa_anim::pose::shared_rig(skeleton).ok()?;
    let cal = Cal::shared(cal).ok()?;
    bind_clip(&rig, &cal, clips, &name)
}

/// The fighting stance of a creature model, when its `.cal` has one that differs from the relaxed idle.
pub fn combat_idle(rig: &Arc<Rig>, cal: &Cal, clips: &ClipLibrary) -> Option<BoundClip> {
    bind_clip(rig, cal, clips, "fist_ba_combat_idle")
}

/// An NPC took or left a pose after it was drawn (`SCUnitModelPostureChanged`; the pose it spawned in is bound with the
/// unit): binds the new clip, or drops the pose when the NPC leaves it.
pub fn posture_changes(
    mut commands: Commands,
    mut events: MessageReader<NetEvent>,
    views: Res<Views>,
    db: Option<Res<Db>>,
    clips: Res<ClipLibrary>,
    mut units: Query<(Option<&mut IdleClips>, &Character)>,
) {
    for NetEvent(event) in events.read() {
        let Event::UnitPosture { obj_id, kind, anim_action, activate } = event else { continue };
        let Some(entity) = views.unit_entity(*obj_id) else { continue };
        let Ok((idle, character)) = units.get_mut(entity) else { continue };
        let action = if *kind == 4 && *activate { *anim_action } else { 0 };
        let pose = db.as_deref().and_then(|db| pose_clip(db, action, &character.assets.skeleton, &character.assets.cal, &clips));
        let tools = db.as_deref().map(|db| db.anim_action_tools(action)).unwrap_or([None, None]);
        match idle {
            Some(mut idle) => {
                idle.pose = pose;
                idle.tools = tools;
                // a pose that changed under a unit that stands in the old one is played from the start
                idle.playing = Which::Relaxed;
            }
            None if pose.is_some() => {
                commands.entity(entity).insert(IdleClips::with_tools(pose, None, tools));
            }
            None => {}
        }
    }
}

/// Starts and ends the combat and pose clips (after locomotion, which restarts a clip only when the unit's state changes).
#[allow(clippy::type_complexity)]
pub fn remote_idles(
    mut commands: Commands,
    time: Res<Time>,
    mut drawn: Query<&mut Drawn>,
    mut units: Query<(Entity, &RemoteUnit, &Motion, &mut Animator, &mut Locomotion, &mut IdleClips, Option<&Acting>), (Without<Dead>, Without<super::seating::SeatPose>)>,
) {
    let now = time.elapsed_secs_f64();
    for (entity, unit, motion, mut animator, mut loco, mut idle, acting) in &mut units {
        let standing = motion.speed <= 0.0 && !motion.airborne && acting.is_none();
        let want = match () {
            _ if !standing => Which::Relaxed,
            _ if unit.combat && idle.combat.is_some() => Which::Combat,
            _ if idle.pose.is_some() => Which::Pose,
            _ => Which::Relaxed,
        };
        // retail: a pose with a tool (the florist's flowerpot) is held with the weapon away; the normal sheathe plays first
        // and the pose starts once it is done
        if want == Which::Pose
            && idle.tools.iter().any(Option::is_some)
            && let Ok(mut drawn) = drawn.get_mut(entity)
            && (drawn.stance(now).is_some() || (drawn.sheathing && now < drawn.until))
        {
            if !drawn.sheathing {
                drawn.until = now;
            }
            commands.entity(entity).try_remove::<PostureTool>();
            continue;
        }
        if want == idle.playing {
            continue;
        }
        let clip = match want {
            Which::Pose => idle.pose.clone(),
            Which::Combat => idle.combat.clone(),
            // standing again: locomotion plays the relaxed idle on its next pass; while moving it already has
            Which::Relaxed => {
                if standing {
                    loco.invalidate();
                }
                None
            }
        };
        if let Some(clip) = clip {
            animator.play(clip, true, 0.25);
            animator.random_idle = None;
        }
        // the pose's tools are in the hands only while the pose plays
        if want == Which::Pose && idle.tools.iter().any(Option::is_some) {
            commands.entity(entity).try_insert(PostureTool { items: idle.tools.clone() });
        } else {
            commands.entity(entity).try_remove::<PostureTool>();
        }
        idle.playing = want;
    }
}
