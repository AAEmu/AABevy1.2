use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use aa_cry::scene::{CryAssets, LoadedModel};
use bevy::mesh::morph::{MeshMorphWeights, MorphAttributes, MorphWeights};
use bevy::prelude::*;

use super::{ExpressionLibrary, FacialSequence};
use crate::{Animator, Cal, CharacterAssets, Error, MorphSet, PartAsset, Result};

/// Opt-in authored facial playback; use with [`crate::spawn_character_with_facial`].
/// Morphs are evaluated on the GPU, before skeletal skinning. No mesh data changes per frame.
pub struct FacialPlugin;

impl Plugin for FacialPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, bind_meshes)
            .add_systems(PostUpdate, (animate_faces.after(crate::animator::animate), refresh_meshes));
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
        if self.follow_body {
            let current = body.and_then(|a| a.current.as_ref());
            let source = current.and_then(|p| p.clip.source.as_deref());
            if source != self.source.as_deref() {
                self.previous.copy_from_slice(out);
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
            self.library.sample_morphs(sequence, self.time, &self.targets, &mut self.scratch);
        }
        self.fade_age += dt;
        let blend = if self.fade_seconds > 0.0 {
            (self.fade_age / self.fade_seconds).clamp(0.0, 1.0)
        } else {
            1.0
        };
        for ((out, value), previous) in out.iter_mut().zip(&self.scratch).zip(&self.previous) {
            *out = previous + (value - previous) * blend;
        }
        // A play/stop during a fade starts from what was displayed, not its unblended target.
        self.scratch.copy_from_slice(out);
    }
}

fn animate_faces(time: Res<Time>, mut faces: Query<(Option<&Animator>, &mut FacialAnimator, &mut MorphWeights)>) {
    for (body, mut face, mut weights) in &mut faces {
        face.update(body, time.delta_secs(), weights.weights_mut());
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

/// Matches neutral position/UV classes and verifies oriented triangles. Exported
/// tangent seams can split equivalent vertices differently; delta equality is checked separately.
pub(crate) fn vertex_map(target: &aa_cry::Model, source: &aa_cry::Model) -> Result<Vec<Vec<usize>>> {
    if target.meshes.len() != 1 || source.meshes.len() != 1 {
        return Err(Error::Format("facial overlay needs one mesh".into()));
    }
    let (a, b) = (&target.meshes[0], &source.meshes[0]);
    if a.uvs.len() != a.positions.len() || b.uvs.len() != b.positions.len() {
        return Err(Error::Format("facial vertex/UV counts differ".into()));
    }
    let matches =
        |i: usize, j: usize| a.positions[i].distance_squared(b.positions[j]) < 1e-10 && a.uvs[i].distance_squared(b.uvs[j]) < 1e-10;
    let candidates: Vec<Vec<usize>> = (0..a.positions.len())
        .map(|i| (0..b.positions.len()).filter(|&j| matches(i, j)).collect())
        .collect();
    if candidates.iter().any(Vec::is_empty) {
        return Err(Error::Format("facial neutral position/UV differs".into()));
    }
    let labels: Vec<u32> = (0..a.positions.len())
        .map(|i| {
            (0..=i)
                .find(|&k| a.positions[i].distance_squared(a.positions[k]) < 1e-10 && a.uvs[i].distance_squared(a.uvs[k]) < 1e-10)
                .unwrap() as u32
        })
        .collect();
    let source_labels: Vec<u32> = (0..b.positions.len())
        .map(|j| {
            (0..a.positions.len())
                .find(|&i| matches(i, j))
                .map(|i| labels[i])
                .ok_or_else(|| Error::Format(format!("no facial position/UV match for vertex {j}")))
        })
        .collect::<Result<_>>()?;
    let triangles = |mesh: &aa_cry::MeshData, mapping: &[u32]| {
        let mut triangles = HashMap::new();
        for subset in &mesh.subsets {
            let range = subset.first_index as usize..(subset.first_index + subset.index_count) as usize;
            for triangle in mesh.indices[range].chunks_exact(3) {
                let v = [
                    mapping[triangle[0] as usize],
                    mapping[triangle[1] as usize],
                    mapping[triangle[2] as usize],
                ];
                let rotations = [v, [v[1], v[2], v[0]], [v[2], v[0], v[1]]];
                *triangles.entry(*rotations.iter().min().unwrap()).or_insert(0usize) += 1;
            }
        }
        triangles
    };
    if triangles(a, &labels) != triangles(b, &source_labels) {
        return Err(Error::Format("facial mapped triangle topology differs".into()));
    }
    Ok(candidates)
}

pub(crate) fn expression_set(path: &str, neutral: &aa_cry::Model) -> Result<Arc<MorphSet>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<MorphSet>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(set) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        return Ok(set.clone());
    }
    let companion = path.replace("_face01.chr", "_face00.chr");
    if companion == path {
        return Err(Error::Missing(format!("expression companion for {path}")));
    }
    let (gender, letter) = if path.contains("/female/") {
        ("female", "f")
    } else {
        ("male", "m")
    };
    // Human race customization heads share Nuian neutral geometry; e.g. the Hariharan female's
    // old face00 is race-shaped already. Only use the shared companion after the same proof.
    let shared = format!("objects/characters/nuian/{gender}/face/nu_{letter}_face00.chr");
    let (source, expression_model, mapping) = [companion, shared]
        .into_iter()
        .find_map(|source| {
            let model = aa_cry::Model::load(&source).ok()?;
            let mapping = vertex_map(neutral, &model).ok()?;
            Some((source, model, mapping))
        })
        .ok_or_else(|| Error::Format(format!("no matching expression topology for {path}")))?;
    let mut set = MorphSet::load(&source)?.ok_or_else(|| Error::Missing(format!("expression morphs in {source}")))?;
    for target in &mut set.targets {
        let mut dense = vec![Vec3::ZERO; expression_model.meshes[0].positions.len()];
        for &(vertex, delta) in &target.deltas {
            *dense
                .get_mut(vertex as usize)
                .ok_or_else(|| Error::Format("facial morph vertex out of range".into()))? = delta;
        }
        let mut deltas = Vec::new();
        for (vertex, candidates) in mapping.iter().enumerate() {
            let delta = dense[candidates[0]];
            if candidates.iter().any(|&i| dense[i].distance_squared(delta) > 1e-12) {
                return Err(Error::Format(format!(
                    "facial seam deltas disagree for {} vertex {vertex}",
                    target.name
                )));
            }
            if delta != Vec3::ZERO {
                deltas.push((vertex as u32, delta));
            }
        }
        target.deltas = deltas;
    }
    let set = Arc::new(set);
    Ok(cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(path.into())
        .or_insert(set)
        .clone())
}

#[cfg(test)]
mod tests {
    use super::*;
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
    let library = ExpressionLibrary::load("animations/pc_expressions.fxl")?;
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
        scratch: vec![0.0; count],
        previous: vec![0.0; count],
        fade_age: 0.0,
    })
}
