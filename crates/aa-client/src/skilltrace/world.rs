//! The systems of the skill trace that read the world: the frame, the roles of the units, clip starts, effects, sounds, network
//! messages and poses (`super`, the hooks table). Added only when the trace is on.

use aa_anim::Animator;
use aa_fx::{ChrEffect, Effect};
use aa_net::state::Target;
use aa_net::{NetCommand, NetEvent};
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;

use super::{emit, emit_at, net, rows};
use crate::ingame::combat::Fx;
use crate::ingame::player::LocalPlayer;
use crate::ingame::units::RemoteUnit;

/// Who the player and its target are this frame, to label the units a row is about.
#[derive(Resource, Default)]
pub struct Roles {
    player: Option<Entity>,
    target: Option<Entity>,
}

impl Roles {
    /// `player` for the local player, `target` for the player's target, else `other`.
    fn of(&self, unit: Entity) -> &'static str {
        if self.player == Some(unit) {
            "player"
        } else if self.target == Some(unit) {
            "target"
        } else {
            "other"
        }
    }
}

fn bits(e: Entity) -> String {
    e.to_bits().to_string()
}

/// Sets the frame number rows carry, finds the player and its target, and writes the `frame` row.
pub fn begin_frame(
    frames: Option<Res<FrameCount>>,
    real: Res<Time<Real>>,
    target: Option<Res<Target>>,
    local: Query<(Entity, &LocalPlayer)>,
    remote: Query<(Entity, &RemoteUnit)>,
    mut roles: ResMut<Roles>,
    mut counted: Local<u32>,
) {
    *counted = counted.wrapping_add(1);
    super::FRAME.store(frames.map_or(*counted, |f| f.0), std::sync::atomic::Ordering::Relaxed);
    roles.player = local.iter().next().map(|(e, _)| e);
    roles.target = target.and_then(|t| t.obj_id).and_then(|obj| {
        // the local player first: a stale remote copy of its object id is not the model the camera follows
        local.iter().find(|(_, l)| l.obj_id == obj).map(|(e, _)| e).or_else(|| remote.iter().find(|(_, r)| r.obj_id == obj).map(|(e, _)| e))
    });
    emit("frame", |r| r.num("dt_ms", real.delta().as_secs_f64() * 1000.0, 2));
}

/// The clips the units' animators started (`aa_anim::trace`), with the `.cal` name where the caller bound it by name.
pub fn drain_anims(roles: Res<Roles>, mut animators: Query<(Entity, &mut Animator, Has<LocalPlayer>, Has<RemoteUnit>)>) {
    for (unit, mut animator, local, remote) in &mut animators {
        if !animator.has_starts() {
            continue;
        }
        let starts = animator.take_starts();
        if !(local || remote) {
            // a preview or a prop: not a unit
            continue;
        }
        let role = roles.of(unit);
        let base_speed = animator.speed;
        for s in starts {
            // the partial-body layer runs in real time, whatever the base clip's speed is
            let speed = if s.layer == aa_anim::trace::UPPER { 1.0 } else { base_speed };
            emit_at("anim", s.wall_us, |r| {
                r.str("inst", &bits(unit))
                    .str("role", role)
                    .opt_str("clip", s.name.as_deref().map(rows::name).as_deref())
                    .opt_str("file", s.file.as_deref().map(rows::path).as_deref())
                    .int("layer", i64::from(s.layer))
                    .num("speed", f64::from(speed), 4)
                    .num("blend", f64::from(s.blend), 4)
                    .flag("loop", s.looping)
            });
        }
    }
}

/// Where an entity is in the world now. A child spawned after this frame's transform propagation has no `GlobalTransform`
/// yet: its parent's, with its own local transform, stand in.
fn world_translation(e: Entity, places: &Query<(&GlobalTransform, &Transform, Option<&ChildOf>)>) -> Vec3 {
    let Ok((global, local, parent)) = places.get(e) else { return Vec3::ZERO };
    if *global != GlobalTransform::IDENTITY {
        return global.translation();
    }
    match parent.and_then(|p| places.get(p.parent()).ok()) {
        Some((parent_global, ..)) => parent_global.transform_point(local.translation),
        None => local.translation,
    }
}

/// The unit an effect is on: the one it names (`Effect::body`), else the nearest ancestor with an animator.
fn unit_of(e: Entity, body: Option<Entity>, parents: &Query<&ChildOf>, animators: &Query<&Animator>) -> Option<Entity> {
    if body.is_some() {
        return body;
    }
    let mut at = e;
    for _ in 0..16 {
        at = parents.get(at).ok()?.parent();
        if animators.contains(at) {
            return Some(at);
        }
    }
    None
}

/// The effects that appeared this frame: every skill, buff and clip effect (placed world effects are left out: they hang on no unit,
/// belong to no fx item, and a torch is not a skill; so are the invisible effects of the shader warm-up).
#[allow(clippy::type_complexity)]
pub fn fx_started(
    roles: Res<Roles>,
    effects: Query<(Entity, &Effect, Option<&Fx>), Added<Effect>>,
    chr: Query<(Entity, &ChrEffect, Option<&Fx>), Added<ChrEffect>>,
    places: Query<(&GlobalTransform, &Transform, Option<&ChildOf>)>,
    parents: Query<&ChildOf>,
    animators: Query<&Animator>,
    names: Query<&Name>,
) {
    let one = |e: Entity, name: String, scale: f32, body: Option<Entity>, fx: Option<&Fx>, wanted: bool| {
        let unit = unit_of(e, body, &parents, &animators);
        if !(wanted || fx.is_some() || unit.is_some()) {
            return;
        }
        // the bone it hangs on (a child of it), or follows (a hit effect that keeps the orientation it was spawned with)
        let bone = unit.and_then(|u| {
            let parent = parents.get(e).ok().map(ChildOf::parent);
            let hung = parent.filter(|p| animators.get(u).is_ok_and(|a| a.bones.contains(p)));
            hung.or_else(|| fx.and_then(|f| f.follow).map(|(_, bone, _)| bone))
        });
        let bone_name = bone.and_then(|b| names.get(b).ok()).map(|n| rows::name(n.as_str()));
        let at = rows::game_pos(world_translation(e, &places));
        emit("fx", |r| {
            r.str("name", &name)
                .str("phase", "spawn")
                .pos("pos", at)
                .opt_str("inst", unit.map(bits).as_deref())
                .opt_str("role", unit.map(|u| roles.of(u)))
                .opt_str("bone", bone_name.as_deref())
                .num("scale", f64::from(scale), 3)
        });
    };
    for (e, effect, fx) in &effects {
        // the warm-up that compiles the pipelines draws effects at no opacity
        if effect.alpha_scale <= 0.0 {
            continue;
        }
        one(e, rows::name(&effect.name), effect.scale, effect.body, fx, effect.item);
    }
    for (e, effect, fx) in &chr {
        one(e, rows::path(&effect.path), effect.scale, None, fx, true);
    }
}

/// The sounds the audio engine was asked to play (`aa_audio::trace`), at the moment it was asked.
pub fn sounds() {
    for s in aa_audio::trace::take() {
        emit_at("sound", s.wall_us, |r| {
            r.str("name", &rows::name(&s.path)).str("phase", "play").opt_pos("pos", s.at.map(rows::game_pos))
        });
    }
}

/// The skill and combat commands out and events in, at the moment the net plugin passes them on (`PreUpdate`: this system is the
/// first thing of `Update` that sees them).
pub fn net_rows(mut commands: MessageReader<NetCommand>, mut events: MessageReader<NetEvent>) {
    let row = |dir: &str, n: net::Net| {
        emit("net", |r| {
            r.str("dir", dir)
                .str("op", n.op)
                .opt_int("skill_id", n.skill_id.map(i64::from))
                .opt_int("tl", n.tl.map(i64::from))
                .opt_int("obj", n.obj.map(i64::from))
                .fields("fields", &n.fields)
        });
    };
    for NetCommand(command) in commands.read() {
        if let Some(n) = net::of_command(command) {
            row("out", n);
        }
    }
    for NetEvent(event) in events.read() {
        if let Some(n) = net::of_event(event) {
            row("in", n);
        }
    }
}

/// A unit moves more than a centimetre or turns more than a tenth of a degree.
fn moved(from: (Vec3, f32), to: (Vec3, f32)) -> bool {
    let turn = (to.1 - from.1 + 540.0).rem_euclid(360.0) - 180.0;
    from.0.distance(to.0) > 0.01 || turn.abs() > 0.1
}

/// The poses of the player and the target, when they change.
pub fn positions(roles: Res<Roles>, places: Query<&GlobalTransform>, mut last: Local<[Option<(Entity, Vec3, f32)>; 2]>) {
    for (i, (unit, entity)) in [("player", roles.player), ("target", roles.target)].into_iter().enumerate() {
        let Some((entity, global)) = entity.and_then(|e| places.get(e).ok().map(|g| (e, g))) else {
            last[i] = None;
            continue;
        };
        let at = aa_world::bevy_to_cry(global.translation());
        let yaw = rows::yaw_degrees(global.rotation());
        // another unit than the last one is a new pose whatever the numbers are
        if last[i].is_some_and(|(e, p, y)| e == entity && !moved((p, y), (at, yaw))) {
            continue;
        }
        last[i] = Some((entity, at, yaw));
        let g = rows::game_pos(global.translation());
        emit("pos", |r| r.str("unit", unit).num("x", g[0], 3).num("y", g[1], 3).num("z", g[2], 3).num("yaw", f64::from(yaw), 2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pose_is_new_after_a_centimetre_or_a_tenth_of_a_degree() {
        let still = (Vec3::new(1.0, 2.0, 3.0), 90.0);
        assert!(!moved(still, (Vec3::new(1.005, 2.0, 3.0), 90.05)));
        assert!(moved(still, (Vec3::new(1.02, 2.0, 3.0), 90.0)));
        assert!(moved(still, (Vec3::new(1.0, 2.0, 3.0), 90.2)));
        // the turn is the short way round the circle
        assert!(!moved((Vec3::ZERO, 179.98), (Vec3::ZERO, -179.98)));
        assert!(moved((Vec3::ZERO, 179.0), (Vec3::ZERO, -179.0)));
    }

    #[test]
    fn roles_name_the_player_and_the_target() {
        let (a, b, c) = (Entity::from_raw_u32(1).unwrap(), Entity::from_raw_u32(2).unwrap(), Entity::from_raw_u32(3).unwrap());
        let roles = Roles { player: Some(a), target: Some(b) };
        assert_eq!((roles.of(a), roles.of(b), roles.of(c)), ("player", "target", "other"));
    }
}
