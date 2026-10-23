//! Authored rigid CGA playback. This deliberately accepts a verified controller subset:
//! constant vector curves and zero-TCB, constant-rate relative axis-angle rotations. Other
//! curves / named ANM clips remain unsupported rather than being replaced by made-up motion.
//!
//! Layout: native CGA Node 0x823 controller references and Controller 0x826. Semantics:
//! CryCGALoader::InitNodes, ControllerTCB::GetOPS, TCBAngleAxisSpline in the public Cry source.

use std::collections::HashMap;
use std::sync::Arc;

use aa_cry::chunk::{ChunkFile, Reader, ty};
use aa_cry::scene::LoadedModel;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::entities::PlacedProp;

#[derive(Clone, Debug)]
struct Spin {
    initial: Quat,
    axis: Vec3,
    rate: f32,
    begin: f32,
    end: f32,
}

impl Spin {
    fn sample(&self, frame: f32) -> Quat {
        let angle = self.rate * (frame.clamp(self.begin, self.end) - self.begin);
        // Max angle-axis keys are relative; Cry accumulates them, then GetOPS conjugates.
        (self.initial * Quat::from_axis_angle(self.axis, angle))
            .conjugate()
            .normalize()
    }
}

#[derive(Clone, Debug, Default)]
struct Channels {
    position: Option<Vec3>,
    rotation: Option<Spin>,
    scale: Option<Vec3>,
}

enum Curve {
    Vector(Vec3),
    Rotation(Spin),
}

#[derive(Debug)]
pub(super) struct Clip {
    start: f32,
    end: f32,
    fps: f32,
    channels: Vec<Option<Channels>>,
}

fn invalid(message: &str) -> aa_cry::Error {
    aa_cry::Error::Unsupported(format!("rigid CGA: {message}"))
}

fn vec3(r: &mut Reader<'_>) -> aa_cry::Result<Vec3> {
    let v = Vec3::new(r.f32()?, r.f32()?, r.f32()?);
    if !v.is_finite() {
        return Err(invalid("nonfinite key"));
    }
    Ok(v)
}

fn curve(data: &[u8], ticks_per_frame: f32) -> aa_cry::Result<Curve> {
    let mut r = Reader::new(data);
    let kind = r.u32()?;
    let count = r.u32()? as usize;
    let flags = r.u32()?;
    r.skip(4)?; // exporter controller ID; node references use the chunk ID
    if count == 0 || count > 16384 {
        return Err(invalid("key count"));
    }
    if matches!(kind, 3 | 6 | 9) {
        // LINEAR3, BEZIER3, TCB3
        let mut value = None;
        for _ in 0..count {
            r.skip(4)?;
            let v = vec3(&mut r)?;
            if value.is_some_and(|a: Vec3| !a.abs_diff_eq(v, 1e-5)) {
                return Err(invalid("varying vector curve"));
            }
            value = Some(v);
            if kind == 6 {
                if vec3(&mut r)?.length_squared() > 1e-12 || vec3(&mut r)?.length_squared() > 1e-12
                {
                    return Err(invalid("nonconstant Bezier tangents"));
                }
            } else if kind == 9 {
                r.skip(20)?; // TCB/ease cannot change a curve whose values are constant
            }
        }
        return Ok(Curve::Vector(value.unwrap()));
    }
    if kind != 10 || flags != 0 {
        return Err(invalid(&format!(
            "controller type {kind}, out-of-range flags {flags}"
        )));
    }
    let mut keys = Vec::with_capacity(count);
    for _ in 0..count {
        let time = r.i32()? as f32 / ticks_per_frame;
        let axis = vec3(&mut r)?;
        let angle = r.f32()?;
        for _ in 0..5 {
            let shape = r.f32()?;
            if !shape.is_finite() || shape.abs() > 1e-7 {
                return Err(invalid("nonzero rotation TCB / ease"));
            }
        }
        if !angle.is_finite() || (angle.abs() > 1e-6 && (axis.length() - 1.0).abs() > 1e-4) {
            return Err(invalid("axis-angle key"));
        }
        keys.push((time, axis, angle));
    }
    let (begin, axis0, angle0) = keys[0];
    let initial = if angle0.abs() < 1e-7 {
        Quat::IDENTITY
    } else {
        Quat::from_axis_angle(axis0.normalize(), angle0)
    };
    let mut axis = Vec3::X;
    let mut rate = 0.0;
    if count > 1 {
        let dt = keys[1].0 - begin;
        if dt <= 0.0 {
            return Err(invalid("nonascending keys"));
        }
        if keys[1].2.abs() > 1e-7 {
            axis = keys[1].1.normalize();
        }
        rate = keys[1].2 / dt;
        for pair in keys.windows(2) {
            let dt = pair[1].0 - pair[0].0;
            let (_, a, angle) = pair[1];
            if dt <= 0.0
                || angle.abs() > std::f32::consts::PI + 1e-5
                || (angle.abs() > 1e-7 && !a.abs_diff_eq(axis, 1e-4))
                || (angle / dt - rate).abs() > 1e-4 * rate.abs().max(1.0)
            {
                return Err(invalid("rotation is not a constant-rate axis-angle curve"));
            }
        }
    }
    Ok(Curve::Rotation(Spin {
        initial,
        axis,
        rate,
        begin,
        end: keys.last().unwrap().0,
    }))
}

impl Clip {
    pub(super) fn load(path: &str, model: &aa_cry::Model) -> aa_cry::Result<Self> {
        Self::parse(
            &aa_pak::Pak::global()
                .read(path)
                .map_err(|e| invalid(&e.to_string()))?,
            model,
        )
    }

    fn parse(data: &[u8], model: &aa_cry::Model) -> aa_cry::Result<Self> {
        let file = ChunkFile::parse(data)?;
        let timing = file.first(ty::TIMING).ok_or_else(|| invalid("no timing"))?;
        if timing.version != 0x918 {
            return Err(invalid("timing version"));
        }
        let mut r = timing.reader();
        let seconds_per_tick = r.f32()?;
        let ticks_per_frame = r.i32()? as f32;
        r.skip(32)?;
        let start = r.i32()? as f32;
        let end = r.i32()? as f32;
        let fps = (seconds_per_tick * ticks_per_frame).recip();
        if !fps.is_finite() || fps <= 0.0 || ticks_per_frame <= 0.0 || end <= start {
            return Err(invalid("timing range"));
        }
        let mut channels = vec![None; model.nodes.len()];
        let mut controllers = HashMap::new();
        // Only rendered LOD0 nodes and their ancestors matter. Unsupported far-LOD tracks
        // must not reject a usable near model; rigid instances currently stay at LOD0.
        let mut relevant = vec![false; model.nodes.len()];
        for mut i in model.lod_nodes(0) {
            for _ in 0..model.nodes.len() {
                relevant[i] = true;
                if let Some(parent) = model.nodes[i].parent {
                    i = parent;
                } else {
                    break;
                }
            }
        }
        for (i, node) in model.nodes.iter().enumerate().filter(|(i, _)| relevant[*i]) {
            let chunk = file
                .get_typed(node.chunk_id, ty::NODE)
                .ok_or_else(|| invalid("missing node"))?;
            if chunk.version != 0x823 {
                return Err(invalid("node version"));
            }
            let mut r = chunk.reader();
            r.seek(188);
            let ids = [r.i32()?, r.i32()?, r.i32()?];
            let mut c = Channels::default();
            for (channel, id) in ids
                .into_iter()
                .enumerate()
                .filter(|(_, id)| *id >= 0 && *id != 0xffff)
            {
                if !controllers.contains_key(&id) {
                    let raw = file
                        .get_typed(id, ty::CONTROLLER)
                        .ok_or_else(|| invalid("missing controller"))?;
                    if raw.version != 0x826 {
                        return Err(invalid("controller version"));
                    }
                    // Some original CGAs were repacked without updating the controller's
                    // copied file offset. The table is authoritative. Recognize the copy by
                    // type/version/id; aa-cry's general chunk reader keeps stale copies.
                    let mut payload = raw.data;
                    if payload.len() >= 16
                        && u32::from_le_bytes(payload[0..4].try_into().unwrap()) == ty::CONTROLLER
                        && u32::from_le_bytes(payload[4..8].try_into().unwrap()) == raw.version
                        && i32::from_le_bytes(payload[12..16].try_into().unwrap()) == id
                    {
                        payload = &payload[16..];
                    }
                    controllers.insert(id, curve(payload, ticks_per_frame)?);
                }
                match (channel, &controllers[&id]) {
                    (0, Curve::Vector(v)) => c.position = Some(*v * 0.01),
                    (1, Curve::Rotation(v)) => c.rotation = Some(v.clone()),
                    (2, Curve::Vector(v)) => c.scale = Some(*v),
                    _ => return Err(invalid("controller/channel mismatch")),
                }
            }
            if c.position.is_some() || c.rotation.is_some() || c.scale.is_some() {
                channels[i] = Some(c);
            }
        }
        if channels.iter().all(Option::is_none) {
            return Err(invalid("no supported channels"));
        }
        Ok(Self {
            start,
            end,
            fps,
            channels,
        })
    }

    fn frame(&self, elapsed: f32, looped: bool) -> f32 {
        let frame = elapsed * self.fps;
        self.start
            + if looped {
                frame.rem_euclid(self.end - self.start)
            } else {
                frame.clamp(0.0, self.end - self.start)
            }
    }

    fn sample(&self, node: usize, frame: f32, mut bind: Transform) -> Transform {
        if let Some(c) = &self.channels[node] {
            if let Some(p) = c.position {
                bind.translation = p;
            }
            if let Some(r) = &c.rotation {
                bind.rotation = r.sample(frame);
            }
            if let Some(s) = c.scale {
                bind.scale = s;
            }
        }
        bind
    }
}

/// Root of a streamed, playing authored rigid entity. Children are its original CGA parts.
#[derive(Component, Clone, Copy, Debug)]
pub struct RigidEntity {
    pub cell: (i32, i32),
    pub guid: u64,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct RigidPart {
    pub cell: (i32, i32),
    pub guid: u64,
}

#[derive(Component)]
struct Playback {
    clip: Arc<Clip>,
    elapsed: f32,
    speed: f32,
    looped: bool,
    targets: Vec<(Entity, usize, Transform)>,
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct RigidSettings {
    /// Pauses existing poses; new instances begin at the authored clip's first frame.
    pub enabled: bool,
}
impl Default for RigidSettings {
    fn default() -> Self {
        Self {
            enabled: std::env::var_os("AA_NO_RIGID_ANIMATION").is_none(),
        }
    }
}

pub(crate) fn build(app: &mut App) {
    app.init_resource::<RigidSettings>()
        .add_systems(Update, update.after(super::stream_objects));
}

fn update(
    time: Res<Time>,
    settings: Res<RigidSettings>,
    mut players: Query<&mut Playback>,
    mut nodes: Query<&mut Transform, Without<Playback>>,
) {
    if !settings.enabled {
        return;
    }
    for mut player in &mut players {
        player.elapsed += time.delta_secs() * player.speed;
        let frame = player.clip.frame(player.elapsed, player.looped);
        for &(entity, index, bind) in &player.targets {
            if let Ok(mut transform) = nodes.get_mut(entity) {
                *transform = player.clip.sample(index, frame, bind);
            }
        }
    }
}

pub(super) fn spawn(
    commands: &mut Commands,
    parent: Entity,
    cell: (i32, i32),
    prop: &PlacedProp,
    loaded: &LoadedModel,
    clip: Arc<Clip>,
    pre: Mat4,
    view: f32,
) -> usize {
    let Some(animation) = &prop.animation else {
        return 0;
    };
    if !animation.name.eq_ignore_ascii_case("default") {
        return 0;
    }
    let root = commands
        .spawn((
            Name::new(prop.name.clone()),
            RigidEntity {
                cell,
                guid: prop.guid,
            },
            Transform::from_matrix(pre),
            Visibility::Inherited,
            ChildOf(parent),
        ))
        .id();
    let mut map = HashMap::new();
    let mut targets = Vec::new();
    fn node_entity(
        commands: &mut Commands,
        loaded: &LoadedModel,
        clip: &Clip,
        root: Entity,
        i: usize,
        map: &mut HashMap<usize, Entity>,
        targets: &mut Vec<(Entity, usize, Transform)>,
        depth: usize,
    ) -> Entity {
        if let Some(&entity) = map.get(&i) {
            return entity;
        }
        let node = &loaded.model.nodes[i];
        let parent = match node.parent {
            Some(p) if depth < 64 => {
                node_entity(commands, loaded, clip, root, p, map, targets, depth + 1)
            }
            _ => root,
        };
        let bind = Transform::from_matrix(Mat4::from(node.local));
        let transform = clip.sample(i, clip.start, bind);
        let entity = commands
            .spawn((
                Name::new(node.name.clone()),
                transform,
                Visibility::Inherited,
                ChildOf(parent),
            ))
            .id();
        if clip.channels[i].is_some() {
            targets.push((entity, i, bind));
        }
        map.insert(i, entity);
        entity
    }
    let mut count = 0;
    for (i, parts) in &loaded.parts {
        let parent = node_entity(commands, loaded, &clip, root, *i, &mut map, &mut targets, 0);
        for part in parts {
            let mut entity = commands.spawn((
                RigidPart {
                    cell,
                    guid: prop.guid,
                },
                Mesh3d(part.mesh.clone()),
                MeshMaterial3d(part.material.clone()),
                part.aabb,
                Transform::IDENTITY,
                super::dissolve::range(view),
                super::distance::ViewDistance::Near,
                ChildOf(parent),
            ));
            if !prop.cast_shadow {
                entity.insert(NotShadowCaster);
            }
            count += 1;
        }
    }
    commands.entity(root).insert(Playback {
        clip,
        elapsed: 0.0,
        speed: animation.speed,
        looped: animation.looped,
        targets,
    });
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_rigid_windmill_preserves_native_turn_timing_and_authored_speed() {
        if !crate::pak_available() {
            return;
        }
        let xml = aa_pak::Pak::global()
            .read_string("worlds/main_world/cells/011_011/client/entities.xml")
            .unwrap();
        let props = crate::entities::parse_animated_entities(&xml);
        let prop = props.iter().find(|p| p.name == "AnimObject30").unwrap();
        assert!(!prop.hidden);
        let animation = prop.animation.as_ref().unwrap();
        assert_eq!(animation.name, "Default");
        assert!(animation.looped && animation.always_update);
        assert!((animation.speed - 0.1).abs() < 1e-6);
        assert!((prop.translation() - Vec3::new(386.02881, 733.87646, 110.51576)).length() < 0.001);
        assert!(
            !crate::entities::parse_static_entities(&xml)
                .iter()
                .any(|p| p.guid == prop.guid)
        );
        let model = aa_cry::Model::load(&prop.model).unwrap();
        let clip = Clip::load(&prop.model, &model).unwrap();
        assert_eq!((clip.start, clip.end), (0.0, 100.0));
        assert!((clip.fps - 30.0).abs() < 0.001);
        let wing = model
            .nodes
            .iter()
            .position(|n| n.name == "mari_windmill_01_wing")
            .unwrap();
        let bind = Transform::from_matrix(Mat4::from(model.nodes[wing].local));
        let p0 = clip.sample(wing, clip.start, bind);
        let p1 = clip.sample(wing, 25.0, bind);
        assert!(
            p0.rotation
                .dot(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                .abs()
                > 0.99999
        );
        assert!(
            p1.rotation
                .dot(Quat::from_rotation_x(-std::f32::consts::PI))
                .abs()
                > 0.99999
        );
        assert!((p1.scale - Vec3::ONE).abs().max_element() < 1e-5);
        assert_eq!(p1.translation, bind.translation);
        assert!((clip.frame(clip.end / clip.fps + 0.5, true) - 15.0).abs() < 1e-4);
        assert_eq!(clip.frame(100.0, false), 100.0);
        let mut world = World::new();
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs_f32(1.0 / 3.0));
        world.insert_resource(time);
        world.insert_resource(RigidSettings { enabled: true });
        let target = world.spawn(bind).id();
        world.spawn(Playback {
            clip: Arc::new(clip),
            elapsed: 0.0,
            speed: animation.speed,
            looped: animation.looped,
            targets: vec![(target, wing, bind)],
        });
        use bevy::ecs::system::RunSystemOnce;
        world.run_system_once(update).unwrap();
        let rotation = world.get::<Transform>(target).unwrap().rotation;
        let expected =
            Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2 - std::f32::consts::TAU / 100.0);
        assert!(
            rotation.dot(expected).abs() > 0.99999,
            "one authored frame after 1/3 s at Speed=.1"
        );
        println!(
            "windmill: {} nodes, {} controlled near nodes, {} frames at {} fps",
            model.nodes.len(),
            world
                .query::<&Playback>()
                .single(&world)
                .unwrap()
                .clip
                .channels
                .iter()
                .filter(|c| c.is_some())
                .count(),
            100,
            30
        );
    }

    #[test]
    fn real_rigid_complex_curve_is_rejected_instead_of_frozen() {
        if !crate::pak_available() {
            return;
        }
        let path = "objects/env/02_harihara/13_has/has_beroe_in_wheel.cga";
        let model = aa_cry::Model::load(path).unwrap();
        assert!(
            Clip::load(path, &model).is_err(),
            "the wheel's many varying TCB tracks need full spline evaluation"
        );
    }
}
