use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aa_cry::scene::{CryAssets, LoadedModel};
use bevy::mesh::morph::{MeshMorphWeights, MorphAttributes, MorphWeights};
use bevy::prelude::*;

use super::bone::BoneChannels;
use super::remap::expression_set;
use super::{ExpressionLibrary, FacialSequence};
use crate::{Animator, Cal, CharacterAssets, Error, PartAsset, Result};

fn head_library(path: &str) -> Result<Arc<ExpressionLibrary>> {
    let head_cal = format!("{}.cal", path.rsplit_once('.').map_or(path, |(stem, _)| stem));
    let head_cal = if aa_pak::Pak::global().exists(&head_cal) { Some(Cal::shared(&head_cal)?) } else { None };
    ExpressionLibrary::load(
        head_cal.as_ref().and_then(|cal| cal.face_library.as_deref()).unwrap_or("animations/pc_expressions.fxl"),
    )
}

/// Opt-in authored facial playback; use with [`crate::spawn_character_with_facial`].
/// Morphs are evaluated on the GPU, before skeletal skinning. No mesh data changes per frame.
pub struct FacialPlugin;

impl Plugin for FacialPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, bind_meshes)
            .add_systems(PostUpdate, (animate_faces.after(crate::animator::animate).before(TransformSystems::Propagate), refresh_meshes));
    }
}

#[derive(Component)]
struct InitialMorphUpload(u8);

fn refresh_meshes(mut commands: Commands, mut meshes: Query<(Entity, &mut Mesh3d, &mut InitialMorphUpload)>) {
    for (entity, mut mesh, mut remaining) in &mut meshes {
        // Bevy 0.20 can upload instance metadata before its morph descriptor is ready.
        // Three initial refreshes fix the observed neutral-face draw; vertices never change.
        mesh.set_changed();
        remaining.0 -= 1;
        if remaining.0 == 0 {
            commands.entity(entity).remove::<InitialMorphUpload>();
        }
    }
}

/// The face's authored expression state. By default it follows the body's matching `.cal`
/// facial entry, including idle variants, pause, playback speed and one-shot return.
#[derive(Component)]
pub struct FacialAnimator {
    pub library: Arc<ExpressionLibrary>,
    pub targets: Vec<String>,
    pub sequence: Option<Arc<FacialSequence>>,
    pub time: f32,
    pub looping: bool,
    pub paused: bool,
    pub follow_body: bool,
    /// Seconds to blend after a change of sequence (also when returning to neutral).
    pub fade_seconds: f32,
    sequences: HashMap<String, String>,
    meshes: Vec<AssetId<Mesh>>,
    source: Option<String>,
    channels: Vec<String>,
    bones: Option<BoneChannels>,
    scratch: Vec<f32>,
    previous: Vec<f32>,
    fade_age: f32,
}

impl FacialAnimator {
    /// Explicit FSQ playback, useful for expressions/talking independent of a body clip.
    /// Authored speech curves are played as supplied; this does not generate lip sync from audio.
    pub fn play(&mut self, sequence: Arc<FacialSequence>, looping: bool) {
        self.previous.clone_from(&self.scratch);
        self.fade_age = 0.0;
        self.time = sequence.start;
        self.sequence = Some(sequence);
        self.looping = looping;
        self.follow_body = false;
        self.paused = false;
    }

    pub fn stop(&mut self) {
        self.previous.clone_from(&self.scratch);
        self.fade_age = 0.0;
        self.sequence = None;
        self.follow_body = false;
    }

    pub fn follow_body(&mut self) {
        self.previous.clone_from(&self.scratch);
        self.fade_age = 0.0;
        self.sequence = None;
        self.follow_body = true;
        // Force resolution even if playback resumes the same CAF after an explicit FSQ.
        self.source = None;
    }

    fn update(&mut self, body: Option<&Animator>, dt: f32, out: &mut [f32]) {
        if let Some(body) = body {
            if self.bones.as_ref().is_none_or(|b| b.rig_id != body.rig.id || !Arc::ptr_eq(&b.library, &self.library)) {
                let bones = BoneChannels::bind(self.library.clone(), &body.rig);
                self.channels.clone_from(&self.targets);
                self.channels.extend(bones.names.iter().cloned());
                self.scratch.resize(self.channels.len(), 0.0);
                self.previous.resize(self.channels.len(), 0.0);
                // Replacing a library starts its own bone state at neutral.
                self.scratch[self.targets.len()..].fill(0.0);
                self.previous[self.targets.len()..].fill(0.0);
                self.bones = Some(bones);
            }
        }
        if self.follow_body {
            let current = body.and_then(|a| a.current.as_ref());
            let source = current.and_then(|p| p.clip.source.as_deref());
            if source != self.source.as_deref() {
                self.previous.clone_from(&self.scratch);
                self.fade_age = 0.0;
                self.source = source.map(str::to_owned);
                self.sequence = source
                    .and_then(|s| self.sequences.get(s))
                    .and_then(|path| match FacialSequence::load(path) {
                        Ok(sequence) => Some(sequence),
                        Err(error) => {
                            warn!("aa-anim: facial sequence {path}: {error}");
                            None
                        }
                    });
            }
            if let Some(current) = current {
                // Do not wrap by FSQ duration: it must remain in phase with the CAF clock.
                self.time = self
                    .sequence
                    .as_ref()
                    .map_or(current.time, |s| (s.start + current.time).clamp(s.start, s.end));
            }
        } else if !self.paused {
            if let Some(sequence) = &self.sequence {
                self.time += dt;
                if self.looping && sequence.duration() > 0.0 {
                    self.time = sequence.start + (self.time - sequence.start).rem_euclid(sequence.duration());
                } else {
                    self.time = self.time.clamp(sequence.start, sequence.end);
                }
            }
        }
        self.scratch.fill(0.0);
        if let Some(sequence) = &self.sequence {
            self.library.sample_weights(sequence, self.time, &self.channels, &mut self.scratch);
        }
        self.fade_age += dt;
        let blend = if self.fade_seconds > 0.0 {
            (self.fade_age / self.fade_seconds).clamp(0.0, 1.0)
        } else {
            1.0
        };
        for (value, previous) in self.scratch.iter_mut().zip(&self.previous) {
            *value = previous + (*value - previous) * blend;
        }
        // A play/stop during a fade starts from what was displayed, not its unblended target.
        out.copy_from_slice(&self.scratch[..self.targets.len()]);
    }
}

fn animate_faces(
    time: Res<Time>,
    mut faces: Query<(Option<&Animator>, &mut FacialAnimator, &mut MorphWeights)>,
    mut transforms: Query<&mut Transform>,
) {
    for (body, mut face, mut weights) in &mut faces {
        if let (Some(body), Some(bones)) = (body, &face.bones) {
            if bones.rig_id == body.rig.id {
                bones.restore(body, &mut transforms);
            }
        }
        face.update(body, time.delta_secs(), weights.weights_mut());
        if let Some(body) = body {
            let face = &mut *face;
            if let Some(bones) = &mut face.bones {
                bones.apply(body, &face.scratch[face.targets.len()..], &mut transforms);
            }
        }
    }
}

fn bind_meshes(mut commands: Commands, meshes: Query<(Entity, &Mesh3d), Added<Mesh3d>>, faces: Query<(Entity, &FacialAnimator)>) {
    for (entity, mesh) in &meshes {
        if let Some((root, _)) = faces.iter().find(|(_, face)| face.meshes.contains(&mesh.0.id())) {
            commands
                .entity(entity)
                .insert((MeshMorphWeights::Reference(root), InitialMorphUpload(3)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_head_cal_selects_library_and_npc_native_expressions() {
        if !aa_cry::pak_available() { return; }
        let head = "objects/characters/people/npc_chef01/face/npc_chef01_face01.chr";
        let head_cal = Cal::shared("objects/characters/people/npc_chef01/face/npc_chef01_face01.cal").unwrap();
        assert_eq!(head_cal.face_library.as_deref(), Some("animations/pc_expressions.fxl"));
        let library = head_library(head).unwrap();
        assert!(Arc::ptr_eq(&library, &ExpressionLibrary::load("animations/pc_expressions.fxl").unwrap()));
        let db = crate::GameDb::open().unwrap();
        for race in crate::Race::ALL {
            for gender in [crate::Gender::Male, crate::Gender::Female] {
                let assets = crate::CharacterDesc::default_for(&db, race, gender).unwrap().resolve(&db).unwrap();
                let face = assets.parts.iter().find(|p| p.kind == crate::PartKind::Face).unwrap();
                assert!(Arc::ptr_eq(&head_library(&face.path).unwrap(), &library), "{}", face.path);
            }
        }
        let model = aa_cry::Model::load(head).unwrap();
        let set = expression_set(head, &model).unwrap();
        assert_eq!(set.targets.len(), 23);
        let native = crate::MorphSet::load(head).unwrap().unwrap();
        assert_eq!(*set, native);
        let targets: Vec<_> = set.targets.iter().map(|t| t.name.clone()).collect();
        let mut weights = vec![0.0; targets.len()];
        let sequence = FacialSequence::load("animations/people/chef_01/facial/fist_ac_talk_11.fsq").unwrap();
        library.sample_morphs(&sequence, 0.5, &targets, &mut weights);
        assert!(weights.iter().any(|w| *w > 0.01), "{weights:?}");
    }
    use crate::{ClipLibrary, SampleOptions, pose::shared_rig};

    #[test]
    fn stopping_during_a_fade_keeps_the_displayed_pose() {
        let library = ExpressionLibrary::parse(
            r#"<ExpressionLibrary><AllEffectors><Effector Name="lid" Type="morph"/></AllEffectors></ExpressionLibrary>"#,
        )
        .unwrap();
        let sequence = Arc::new(
            FacialSequence::parse(r#"<FacialSequence EndTime="1"><Channel Name="lid"><Spline Keys="0:1:0,"/></Channel></FacialSequence>"#)
                .unwrap(),
        );
        let mut face = FacialAnimator {
            library: Arc::new(library),
            targets: vec!["lid".into()],
            sequence: None,
            time: 0.0,
            looping: true,
            paused: false,
            follow_body: false,
            fade_seconds: 1.0,
            sequences: HashMap::new(),
            meshes: Vec::new(),
            source: None,
            channels: vec!["lid".into()],
            bones: None,
            scratch: vec![0.0],
            previous: vec![0.0],
            fade_age: 0.0,
        };
        let mut out = [0.0];
        face.play(sequence, false);
        face.update(None, 0.25, &mut out);
        assert_eq!(out, [0.25]);
        face.stop();
        face.update(None, 0.0, &mut out);
        assert_eq!(out, [0.25]);
        face.update(None, 0.5, &mut out);
        assert_eq!(out, [0.125]);
        face.update(None, 0.5, &mut out);
        assert_eq!(out, [0.0]);
    }

    #[test]
    fn follows_body_pause_unmatched_stop_and_explicit_sequence() {
        if !aa_cry::pak_available() {
            return;
        }
        let base = "animations/nuian/male/";
        let rig = shared_rig("objects/characters/nuian/male/nude/nu_m_base.chr").unwrap();
        let bound = ClipLibrary::default()
            .bound(&rig, &format!("{base}fist/fist_ac_stage_idle.caf"), SampleOptions::default())
            .unwrap();
        let mut body = Animator::new(rig, Vec::new());
        body.play(bound.clone(), true, 0.0);
        body.current.as_mut().unwrap().time = 1.08;
        body.paused = true;
        let mut face = FacialAnimator {
            library: ExpressionLibrary::load(&format!("{base}facial/nuian_m.fxl")).unwrap(),
            targets: vec!["eye_close_l".into(), "m_a".into()],
            sequence: None,
            time: 0.0,
            looping: true,
            paused: false,
            follow_body: true,
            fade_seconds: 0.0,
            sequences: HashMap::from([(
                bound.source.as_ref().unwrap().to_string(),
                format!("{base}facial/fist_ac_stage_idle.fsq"),
            )]),
            meshes: Vec::new(),
            source: None,
            channels: vec!["eye_close_l".into(), "m_a".into()],
            bones: None,
            scratch: vec![0.0; 2],
            previous: vec![0.0; 2],
            fade_age: 0.0,
        };
        let mut out = [0.0; 2];
        face.update(Some(&body), 0.01, &mut out);
        assert!(out[0] > 0.99);
        body.update(2.0);
        face.update(Some(&body), 2.0, &mut out);
        assert_eq!(face.time, 1.08);
        assert!(out[0] > 0.99);

        let mut unmatched = bound.clone();
        unmatched.source = Some("unmatched.caf".into());
        body.play(unmatched, true, 0.0);
        face.update(Some(&body), 0.01, &mut out);
        assert_eq!(out, [0.0; 2]);
        face.play(FacialSequence::load(&format!("{base}facial/default.fsq")).unwrap(), true);
        face.time = 3.5;
        face.paused = true;
        face.update(Some(&body), 1.0, &mut out);
        assert!(out[1] > 0.85);
        face.stop();
        face.update(Some(&body), 0.01, &mut out);
        assert_eq!(out, [0.0; 2]);
        face.play(FacialSequence::load(&format!("{base}facial/default.fsq")).unwrap(), true);
        body.current = None;
        face.follow_body();
        face.update(Some(&body), 0.01, &mut out);
        assert_eq!(out, [0.0; 2]);
        body.play(bound, true, 0.0);
        body.current.as_mut().unwrap().time = 1.08;
        face.update(Some(&body), 0.01, &mut out);
        assert!(out[0] > 0.99);
    }

    #[test]
    fn bones_restore_the_body_through_pause_interval_stop_and_library_change() {
        if !aa_cry::pak_available() {
            return;
        }
        use std::time::Duration;
        let rig = shared_rig("objects/characters/nuian/male/nude/nu_m_base.chr").unwrap();
        let head = rig.skeleton.bones.iter().position(|b| b.name == "Bip01 Head").unwrap();
        let mut app = App::new();
        app.insert_resource(Time::<()>::default()).add_plugins((bevy::transform::TransformPlugin, crate::AnimPlugin, FacialPlugin));
        let entity = app.world_mut().spawn((Transform::default(), GlobalTransform::default())).id();
        let mut entities = vec![Entity::PLACEHOLDER; rig.len()];
        entities[head] = entity;
        let mut body = Animator::new(rig.clone(), entities);
        body.play(
            ClipLibrary::default().bound(&rig, "animations/nuian/male/fist/fist_ac_stage_idle.caf", SampleOptions::default()).unwrap(),
            true,
            0.0,
        );
        body.update(0.7);
        body.paused = true;
        body.interval = 1000.0;
        let body_rotation = body.pose().rot[head];
        let body_position = body.pose().pos[head];
        assert!(body_rotation.angle_between(rig.bind.rot[head]) > 0.001);
        let mut face = FacialAnimator {
            library: ExpressionLibrary::load("animations/pc_expressions.fxl").unwrap(),
            targets: vec!["eye_close_l".into()],
            sequence: None,
            time: 0.0,
            looping: false,
            paused: true,
            follow_body: false,
            fade_seconds: 0.0,
            sequences: HashMap::new(),
            meshes: Vec::new(),
            source: None,
            channels: vec!["eye_close_l".into()],
            bones: None,
            scratch: vec![0.0],
            previous: vec![0.0],
            fade_age: 0.0,
        };
        let sequence = Arc::new(
            FacialSequence::parse(r#"<FacialSequence EndTime="1"><Channel Name="head_x"><Spline Keys="0:0.5:0,"/></Channel></FacialSequence>"#)
                .unwrap(),
        );
        face.play(sequence.clone(), false);
        face.paused = true;
        let root = app.world_mut().spawn((body, face, MorphWeights::new(vec![0.0], None).unwrap())).id();
        let step = |app: &mut App, dt| {
            app.world_mut().resource_mut::<Time>().advance_by(Duration::from_secs_f32(dt));
            app.update();
        };
        step(&mut app, 0.01);
        let displayed = *app.world().get::<Transform>(entity).unwrap();
        assert!(displayed.rotation.angle_between(body_rotation) > 0.1);
        // Transform propagation must see the new facial bone in the same frame.
        let global = app.world().get::<GlobalTransform>(entity).unwrap().to_scale_rotation_translation();
        assert!(global.1.abs_diff_eq(displayed.rotation, 1e-6));
        for _ in 0..8 {
            step(&mut app, 0.01);
        }
        assert_eq!(*app.world().get::<Transform>(entity).unwrap(), displayed);
        // Removing the current body clip still restores from its stored pose, without adding
        // the facial offset to last frame's Transform.
        app.world_mut().get_mut::<Animator>(root).unwrap().current = None;
        for _ in 0..8 {
            step(&mut app, 0.01);
        }
        assert_eq!(*app.world().get::<Transform>(entity).unwrap(), displayed);
        {
            let mut face = app.world_mut().get_mut::<FacialAnimator>(root).unwrap();
            face.fade_seconds = 0.2;
            face.stop();
        }
        step(&mut app, 0.1);
        assert_ne!(app.world().get::<Transform>(entity).unwrap().rotation, displayed.rotation);
        step(&mut app, 0.1);
        let restored = app.world().get::<Transform>(entity).unwrap();
        assert_eq!((restored.rotation, restored.translation), (body_rotation, body_position));
        {
            let mut face = app.world_mut().get_mut::<FacialAnimator>(root).unwrap();
            face.fade_seconds = 0.0;
            face.play(sequence, false);
        }
        step(&mut app, 0.01);
        app.world_mut().get_mut::<FacialAnimator>(root).unwrap().library = Arc::new(ExpressionLibrary::default());
        step(&mut app, 0.01);
        let restored = app.world().get::<Transform>(entity).unwrap();
        assert_eq!((restored.rotation, restored.translation), (body_rotation, body_position));
    }
}

/// Called before Bevy extracts the just-loaded meshes. Handles remain those of the loaded face,
/// so materials, static customization, joint mapping and entity hierarchy are untouched.
pub(crate) fn prepare_face(
    cry: &mut CryAssets,
    assets: &CharacterAssets,
    part: &PartAsset,
    loaded: &LoadedModel,
) -> Result<FacialAnimator> {
    let set = expression_set(&part.path, &loaded.model)?;
    if set.targets.len() > bevy::mesh::morph::MAX_MORPH_WEIGHTS {
        return Err(Error::Format("too many facial targets".into()));
    }
    // The playable heads use unprefixed morphs. The older race-specific libraries contain
    // el_m_*/fe_m_* morphs and close_l controllers that do not match these meshes/current FSQs.
    let library = head_library(&part.path)?;
    let cal = Cal::shared(&assets.cal)?;
    let sequences = cal
        .animations
        .iter()
        .filter_map(|(name, body)| cal.facial.get(name).map(|face| (body.path.clone(), face.path.clone())))
        .collect();
    let targets: Vec<_> = set.targets.iter().map(|t| t.name.clone()).collect();
    let mut meshes = Vec::new();
    let mut prepared = Vec::new();
    for (ni, parts) in &loaded.parts {
        let source = &loaded.model.meshes[loaded.model.nodes[*ni]
            .mesh
            .ok_or_else(|| Error::Format("facial node without mesh".into()))?];
        // aa-cry splits each material subset and assigns local vertex indices in order of
        // first appearance. Reproduce that remap; original morph indices address the full mesh.
        let subsets: Vec<_> = source.subsets.iter().filter(|s| s.index_count > 0).collect();
        if subsets.len() != parts.len() {
            return Err(Error::Format("facial material/subset count differs".into()));
        }
        for (subset, part) in subsets.into_iter().zip(parts) {
            let range = subset.first_index as usize..(subset.first_index + subset.index_count) as usize;
            let mut seen = HashSet::new();
            let order: Vec<_> = source.indices[range].iter().copied().filter(|i| seen.insert(*i)).collect();
            let mut attributes = Vec::with_capacity(order.len() * targets.len());
            for target in &set.targets {
                let mut dense = vec![Vec3::ZERO; source.positions.len()];
                for &(vertex, delta) in &target.deltas {
                    let Some(out) = dense.get_mut(vertex as usize) else {
                        return Err(Error::Format("facial morph vertex out of range".into()));
                    };
                    *out = delta;
                }
                attributes.extend(
                    order
                        .iter()
                        .map(|&i| MorphAttributes::new(dense[i as usize], Vec3::ZERO, Vec3::ZERO)),
                );
            }
            let mesh = cry
                .meshes
                .get(&part.mesh)
                .ok_or_else(|| Error::Missing("facial mesh already extracted".into()))?;
            if mesh
                .try_attribute(Mesh::ATTRIBUTE_POSITION)
                .map_err(|e| Error::Format(e.to_string()))?
                .len()
                != order.len()
            {
                return Err(Error::Format("facial subset vertex remap differs".into()));
            }
            prepared.push((part.mesh.clone(), attributes));
            meshes.push(part.mesh.id());
        }
    }
    for (handle, attributes) in prepared {
        let mut mesh = cry
            .meshes
            .get_mut(&handle)
            .ok_or_else(|| Error::Missing("facial mesh already extracted".into()))?;
        mesh.try_set_morph_targets(attributes).map_err(|e| Error::Format(e.to_string()))?;
        mesh.set_morph_target_names(targets.clone());
    }
    let count = targets.len();
    let channels = targets.clone();
    Ok(FacialAnimator {
        library,
        targets,
        sequence: None,
        time: 0.0,
        looping: true,
        paused: false,
        follow_body: true,
        fade_seconds: 0.08,
        sequences,
        meshes,
        source: None,
        channels,
        bones: None,
        scratch: vec![0.0; count],
        previous: vec![0.0; count],
        fade_age: 0.0,
    })
}
