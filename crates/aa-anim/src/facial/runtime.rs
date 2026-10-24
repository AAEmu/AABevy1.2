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

#[derive(Component)]
struct AttachedCharacterFace(Vec<(Entity, Handle<Mesh>)>);

/// Enable authored expressions on an already spawned ordinary `Character` in place.
/// Clones its rendered face subsets, or rebuilds extracted CPU data with the same
/// Character face-morph recipe/converter. Preserves customization, materials, joints
/// and all other parts. Returns false when this character is already enabled.
/// Call only when eligibility changes, not each frame. Requires `FacialPlugin`.
pub fn enable_character_face(world: &mut World, root: Entity) -> Result<bool> {
    if world.get::<AttachedCharacterFace>(root).is_some() { return Ok(false); }
    if world.get::<FacialAnimator>(root).is_some() || world.get::<MorphWeights>(root).is_some() {
        return Err(Error::Format("character already owns facial playback".into()));
    }
    let character = world.get::<crate::Character>(root)
        .ok_or_else(|| Error::Missing("spawned Character".into()))?.clone();
    if world.get::<Animator>(root).is_none() {
        return Err(Error::Missing("awake character Animator".into()));
    }
    if character.parts.len() != character.assets.parts.len() {
        return Err(Error::Format("spawned character part indexing differs".into()));
    }
    let index = character.assets.parts.iter().position(|p| p.kind == crate::PartKind::Face)
        .ok_or_else(|| Error::Missing("character face".into()))?;
    let part = &character.assets.parts[index];
    let face_root = character.parts[index];
    let source = world.get::<aa_cry::scene::CryModel>(face_root)
        .ok_or_else(|| Error::Missing("spawned face CryModel".into()))?.clone();
    if crate::cdf::norm(&source.path) != crate::cdf::norm(&part.path) || source.model.meshes.len() != 1 {
        return Err(Error::Format("spawned face source differs".into()));
    }
    // aa-cry spawn_loaded emits skinned subsets directly under the model root,
    // in LOD0 node/subset order. Reject other hierarchy/layouts before allocation.
    let children = world.get::<Children>(face_root)
        .ok_or_else(|| Error::Missing("spawned face subsets".into()))?;
    let subsets = children.iter().filter_map(|entity| world.get::<Mesh3d>(entity).map(|m| (entity, m.0.clone()))).collect::<Vec<_>>();
    if subsets.iter().any(|(entity, _)| world.get::<bevy::mesh::skinning::SkinnedMesh>(*entity).is_none()
        || world.get::<MeshMorphWeights>(*entity).is_some()) {
        return Err(Error::Format("face subsets are not ordinary skinned meshes".into()));
    }
    let nodes = source.model.lod_nodes(0).into_iter().filter(|&i| source.model.nodes[i].mesh.is_some()).collect::<Vec<_>>();
    if nodes.len() != 1 { return Err(Error::Format("face needs one rendered node".into())); }
    let parts = vec![(nodes[0], subsets.iter().map(|(_, handle)| handle.clone()).collect())];
    let mut state = bevy::ecs::system::SystemState::<CryAssets>::new(world);
    let (handles, mut face, weights) = {
        let mut cry = state.get_mut(world).map_err(|error| Error::Format(error.to_string()))?;
        {
            // aa-cry normally releases CPU vertices after upload. Rebuild only those
            // selected subsets using its same converter and the character's same static
            // face-morph hook, retaining the already styled material and skin components.
            let geometry = crate::character::face_geometry(&mut cry, &character.assets, part, &source.model)?;
            let data = &geometry.meshes[0];
            let mut rebuilt = HashMap::new();
            let source_subsets = data.subsets.iter().filter(|s| s.index_count > 0).cloned().collect::<Vec<_>>();
            if source_subsets.len() != subsets.len() {
                return Err(Error::Format("spawned face subset count differs".into()));
            }
            let needs_rebuild = subsets.iter().any(|(_, handle)| cry.meshes.get(handle).is_some_and(|mesh| mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).is_err()));
            if needs_rebuild {
                for ((_, handle), subset) in subsets.iter().zip(&source_subsets) {
                    let mesh = cry.meshes.get(handle).ok_or_else(|| Error::Missing("rendered face mesh".into()))?;
                    if mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).is_err() {
                        let (mesh, _) = aa_cry::scene::subset_mesh(&data, subset, true, bevy::asset::RenderAssetUsages::RENDER_WORLD)
                            .ok_or_else(|| Error::Format("cannot rebuild rendered face subset".into()))?;
                        rebuilt.insert(handle.id(), mesh);
                    }
                }
            }
            prepare_private_subsets(&mut cry, &part.path, &character.assets.cal, (&source.model, &geometry), &parts, Some(&rebuilt))?
        }
    };
    face.meshes = face.meshes.iter().map(|id| handles[id].id()).collect();
    for (entity, original) in &subsets {
        world.entity_mut(*entity).insert((Mesh3d(handles[&original.id()].clone()), MeshMorphWeights::Reference(root), InitialMorphUpload(3)));
    }
    world.entity_mut(root).insert((face, weights, AttachedCharacterFace(subsets)));
    Ok(true)
}

/// Undo only an in-place enable made by `enable_character_face`.
/// Restores original subset handles and underlying bone poses, including asleep bodies.
/// Existing spawn-time/local-player facial playback is left alone. Returns false if absent.
pub fn disable_character_face(world: &mut World, root: Entity) -> bool {
    let Ok(mut entity) = world.get_entity_mut(root) else { return false };
    let Some(attached) = entity.take::<AttachedCharacterFace>() else { return false };
    let face = entity.take::<FacialAnimator>();
    entity.remove::<MorphWeights>();
    if let Some(bones) = face.as_ref().and_then(|face| face.bones.as_ref()) { bones.detach(world); }
    for (subset, original) in attached.0 {
        if let Ok(mut entity) = world.get_entity_mut(subset) {
            entity.insert(Mesh3d(original));
            entity.remove::<(MeshMorphWeights, InitialMorphUpload)>();
        }
    }
    true
}

pub(crate) fn character_face_enabled(world: &World, root: Entity) -> Result<bool> {
    let face = world.get::<FacialAnimator>(root);
    let weights = world.get::<MorphWeights>(root);
    if face.is_some() != weights.is_some()
        || (world.get::<AttachedCharacterFace>(root).is_some() && face.is_none())
        || face.zip(weights).is_some_and(|(f, w)| f.targets.len() != w.weights().len())
    { return Err(Error::Format("inconsistent existing character facial state".into())); }
    Ok(face.is_some())
}

pub(crate) fn validate_replacement_face(world: &World, root: Entity, rig_id: u64, next: &FacialAnimator) -> Result<()> {
    let previous = world.get::<FacialAnimator>(root).ok_or_else(|| Error::Missing("existing facial animator".into()))?;
    if previous.targets != next.targets || !Arc::ptr_eq(&previous.library, &next.library)
        || previous.sequences != next.sequences || previous.bones.as_ref().is_some_and(|b| b.rig_id != rig_id)
    { return Err(Error::Format("replacement facial targets, library or rig are incompatible".into())); }
    Ok(())
}

pub(crate) fn rebind_character_face(world: &mut World, root: Entity, part: Entity, next: FacialAnimator, originals: Vec<Handle<Mesh>>) {
    let subsets = world.get::<Children>(part).unwrap().iter().filter(|&e| world.get::<Mesh3d>(e).is_some()).collect::<Vec<_>>();
    debug_assert_eq!(subsets.len(), originals.len());
    world.get_mut::<FacialAnimator>(root).unwrap().meshes = next.meshes;
    for &subset in &subsets {
        world.entity_mut(subset).insert((MeshMorphWeights::Reference(root), InitialMorphUpload(3)));
    }
    if let Some(mut attached) = world.get_mut::<AttachedCharacterFace>(root) {
        attached.0 = subsets.into_iter().zip(originals).collect();
    }
}

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
    #[cfg(test)]
    pub(crate) fn playback_snapshot(&self) -> String {
        format!("{:?}", (Arc::as_ptr(&self.library), &self.targets, self.sequence.as_ref().map(Arc::as_ptr),
            (self.time, self.looping, self.paused, self.follow_body, self.fade_seconds),
            &self.source, &self.channels, &self.bones, &self.scratch, &self.previous, self.fade_age))
    }

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
    fn hood_cut_refresh_preserves_expression_state_and_retires_private_meshes() {
        if !aa_cry::pak_available() { return; }
        use crate::character::refresh_tests::{app, appearance, spawn, replace, subsets};
        let mut app = app();
        let mut assets = appearance(&[]);
        assets.desc.customization.modifier.clear();
        let root = spawn(&mut app, assets, false);
        let other = spawn(&mut app, appearance(&[]), true);
        app.update();
        let other_meshes = subsets(app.world(), other).iter().map(|&e| app.world().get::<Mesh3d>(e).unwrap().0.id()).collect::<Vec<_>>();
        // Enabling after renderer extraction must rebuild the same styled geometry.
        for entity in subsets(app.world(), root) {
            let handle = app.world().get::<Mesh3d>(entity).unwrap().0.clone();
            app.world_mut().resource_mut::<Assets<Mesh>>().get_mut(&handle).unwrap().take_gpu_data().unwrap();
        }
        assert!(enable_character_face(app.world_mut(), root).unwrap());
        let sequence = FacialSequence::load("animations/nuian/female/facial/fist_ac_talk_11.fsq").unwrap();
        {
            let mut face = app.world_mut().get_mut::<FacialAnimator>(root).unwrap();
            face.play(sequence, false); face.time = 0.51; face.paused = true; face.fade_seconds = 0.4;
        }
        app.update();
        let before = app.world().get::<FacialAnimator>(root).unwrap().playback_snapshot();
        let weights = app.world().get::<MorphWeights>(root).unwrap().weights().to_vec();
        let mut hood = appearance(&[18230]); hood.desc.customization.modifier.clear();
        let face_part = hood.parts.iter().find(|p| p.kind == crate::PartKind::Face).unwrap().clone();
        let neutral = aa_cry::Model::load(&face_part.path).unwrap();
        let set = expression_set(&face_part.path, &neutral).unwrap();
        let geometry = {
            let mut state = bevy::ecs::system::SystemState::<CryAssets>::new(app.world_mut());
            let mut cry = state.get_mut(app.world_mut()).unwrap();
            crate::character::face_geometry(&mut cry, &hood, &face_part, &neutral).unwrap()
        };
        let triangles = |model: &aa_cry::Model| model.meshes[0].indices.as_chunks::<3>().0.iter().filter(|t| t[0] == t[1] && t[1] == t[2]).count();
        assert_eq!(triangles(&geometry) - triangles(&neutral), 370);
        let old_ids = subsets(app.world(), root).iter().map(|&e| app.world().get::<Mesh3d>(e).unwrap().0.id()).collect::<Vec<_>>();
        replace(app.world_mut(), root, hood).unwrap();
        assert_eq!(before, app.world().get::<FacialAnimator>(root).unwrap().playback_snapshot());
        assert_eq!(weights, app.world().get::<MorphWeights>(root).unwrap().weights());
        let entities = subsets(app.world(), root);
        for ((entity, subset), index) in entities.iter().zip(&geometry.meshes[0].subsets).zip(0..) {
            let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(*entity).unwrap().0).unwrap();
            let data = &geometry.meshes[0];
            let range = subset.first_index as usize..(subset.first_index + subset.index_count) as usize;
            let mut seen = HashSet::new();
            let order = data.indices[range].iter().copied().filter(|i| seen.insert(*i)).collect::<Vec<_>>();
            if index == 0 { assert_eq!(order.len(), 2195); }
            let attributes = mesh.get_morph_targets().unwrap();
            for (target, actual) in set.targets.iter().zip(attributes.chunks_exact(order.len())) {
                let sparse = target.deltas.iter().copied().collect::<HashMap<_,_>>();
                for (&vertex, delta) in order.iter().zip(actual) {
                    assert_eq!(delta.position, sparse.get(&vertex).copied().unwrap_or(Vec3::ZERO));
                }
            }
            assert!(matches!(app.world().get::<MeshMorphWeights>(*entity), Some(MeshMorphWeights::Reference(e)) if *e == root));
        }
        let private_ids = entities.iter().map(|&e| app.world().get::<Mesh3d>(e).unwrap().0.id()).collect::<Vec<_>>();
        assert!(disable_character_face(app.world_mut(), root));
        let originals = entities.iter().map(|&e| app.world().get::<Mesh3d>(e).unwrap().0.clone()).collect::<Vec<_>>();
        assert_eq!(app.world().resource::<Assets<Mesh>>().get(&originals[0]).unwrap().count_vertices(), 2195);
        let baseline = originals.iter().map(|h| app.world().resource::<Assets<Mesh>>().get(h).unwrap().clone()).collect::<Vec<_>>();
        app.update();
        for id in old_ids.iter().chain(&private_ids) { assert!(!app.world().resource::<Assets<Mesh>>().contains(*id)); }
        for handle in &originals { app.world_mut().resource_mut::<Assets<Mesh>>().get_mut(handle).unwrap().take_gpu_data().unwrap(); }
        assert!(enable_character_face(app.world_mut(), root).unwrap());
        for (entity, expected) in entities.iter().zip(&baseline) {
            let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(*entity).unwrap().0).unwrap();
            for (attribute, values) in expected.attributes() { assert_eq!(mesh.try_attribute(attribute.id).unwrap(), values); }
            assert_eq!(format!("{:?}",mesh.indices()),format!("{:?}",expected.indices()));
        }
        assert!(disable_character_face(app.world_mut(), root));
        drop(originals);
        drop(baseline);
        let mut retired = Vec::new();
        for items in [&[22874][..], &[18230], &[], &[18230], &[22874], &[]] {
            assert!(enable_character_face(app.world_mut(), root).unwrap());
            retired.extend(subsets(app.world(), root).iter().map(|&e| app.world().get::<Mesh3d>(e).unwrap().0.id()));
            replace(app.world_mut(), root, appearance(items)).unwrap();
            retired.extend(subsets(app.world(), root).iter().map(|&e| app.world().get::<Mesh3d>(e).unwrap().0.id()));
            assert!(disable_character_face(app.world_mut(), root));
            app.update();
        }
        assert!(retired.iter().all(|id| !app.world().resource::<Assets<Mesh>>().contains(*id)));
        for id in other_meshes { assert!(app.world().resource::<Assets<Mesh>>().contains(id)); }
        let final_ids = subsets(app.world(), root).iter().map(|&e| app.world().get::<Mesh3d>(e).unwrap().0.id()).collect::<Vec<_>>();
        app.world_mut().despawn(root); app.update();
        assert!(final_ids.iter().all(|id| !app.world().resource::<Assets<Mesh>>().contains(*id)));
    }


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
    fn spawned_character_face_toggles_preserve_customized_subsets_and_release_clones() {
        if !aa_cry::pak_available() { return; }
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), aa_cry::scene::CryPlugin, crate::AnimPlugin, FacialPlugin));
        app.init_asset::<Mesh>();
        app.init_asset::<Image>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
        let db = crate::GameDb::open().unwrap();
        let mut desc = crate::CharacterDesc::default_for(&db, crate::Race::Nuian, crate::Gender::Male).unwrap();
        desc.hair = None;
        desc.costume = crate::Costume::None;
        let assets = desc.resolve(&db).unwrap();
        let mut state = bevy::ecs::system::SystemState::<(Commands, CryAssets, Res<ClipLibrary>)>::new(app.world_mut());
        let (first, other) = {
            let (mut commands, mut cry, clips) = state.get_mut(app.world_mut()).unwrap();
            let first = crate::spawn_character(&mut commands, &mut cry, &clips, assets.clone(), Transform::default()).unwrap();
            let other = crate::spawn_character(&mut commands, &mut cry, &clips, assets, Transform::default()).unwrap();
            (first, other)
        };
        state.apply(app.world_mut());
        app.update();
        let face_root = {
            let character = app.world().get::<crate::Character>(first).unwrap();
            character.parts[character.assets.parts.iter().position(|p| p.kind == crate::PartKind::Face).unwrap()]
        };
        let subsets = app.world().get::<Children>(face_root).unwrap().iter()
            .filter(|entity| app.world().get::<Mesh3d>(*entity).is_some()).collect::<Vec<_>>();
        let original = subsets.iter().map(|entity| app.world().get::<Mesh3d>(*entity).unwrap().0.clone()).collect::<Vec<_>>();
        let materials = subsets.iter().map(|entity| app.world().get::<MeshMaterial3d<StandardMaterial>>(*entity).unwrap().0.clone()).collect::<Vec<_>>();
        let baseline = original.iter().map(|handle| app.world().resource::<Assets<Mesh>>().get(handle).unwrap().clone()).collect::<Vec<_>>();
        // Represents static face customization already applied by the character loader.
        let positions = {
            let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
            let mut mesh = meshes.get_mut(&original[0]).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(positions) = mesh.try_attribute_mut(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
            positions[0][0] += 0.001;
            positions.clone()
        };
        let character = app.world().get::<crate::Character>(first).unwrap().clone();
        app.world_mut().get_mut::<crate::Character>(first).unwrap().parts.clear();
        let before = app.world().resource::<Assets<Mesh>>().len();
        assert!(enable_character_face(app.world_mut(), first).is_err());
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), before);
        app.world_mut().entity_mut(first).insert(character);
        for _ in 0..2 {
            assert!(enable_character_face(app.world_mut(), first).unwrap());
            let clones = subsets.iter().map(|entity| app.world().get::<Mesh3d>(*entity).unwrap().0.clone()).collect::<Vec<_>>();
            assert!(!enable_character_face(app.world_mut(), first).unwrap());
            assert!(app.world().get::<FacialAnimator>(other).is_none());
            for (i, entity) in subsets.iter().enumerate() {
                assert_ne!(clones[i], original[i]);
                assert_eq!(app.world().get::<MeshMaterial3d<StandardMaterial>>(*entity).unwrap().0, materials[i]);
                assert!(matches!(app.world().get::<MeshMorphWeights>(*entity), Some(MeshMorphWeights::Reference(root)) if *root == first));
            }
            let mesh = app.world().resource::<Assets<Mesh>>().get(&clones[0]).unwrap();
            assert_eq!(mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).unwrap(), &bevy::mesh::VertexAttributeValues::Float32x3(positions.clone()));
            assert!(disable_character_face(app.world_mut(), first));
            assert!(!disable_character_face(app.world_mut(), first));
            for (i, entity) in subsets.iter().enumerate() {
                assert_eq!(app.world().get::<Mesh3d>(*entity).unwrap().0, original[i]);
                assert!(app.world().get::<MeshMorphWeights>(*entity).is_none());
            }
            let ids = clones.iter().map(Handle::id).collect::<Vec<_>>();
            drop(clones);
            app.update();
            assert!(ids.iter().all(|id| !app.world().resource::<Assets<Mesh>>().contains(*id)));
        }
        // Simulate real renderer extraction after restoring the authored character recipe.
        // The late path must reproduce every subset attribute/index, not just positions.
        {
            let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
            for (handle, mesh) in original.iter().zip(&baseline) {
                meshes.insert(handle.id(), mesh.clone()).unwrap();
                meshes.get_mut(handle).unwrap().take_gpu_data().unwrap();
            }
        }
        assert!(enable_character_face(app.world_mut(), first).unwrap());
        for (i, entity) in subsets.iter().enumerate() {
            let mesh = app.world().resource::<Assets<Mesh>>().get(&app.world().get::<Mesh3d>(*entity).unwrap().0).unwrap();
            for (attribute, values) in baseline[i].attributes() {
                assert_eq!(mesh.try_attribute(attribute.id).unwrap(), values);
            }
            assert_eq!(format!("{:?}", mesh.indices()), format!("{:?}", baseline[i].indices()));
        }
        let late_ids = subsets.iter().map(|entity| app.world().get::<Mesh3d>(*entity).unwrap().0.id()).collect::<Vec<_>>();
        assert!(disable_character_face(app.world_mut(), first));
        for (i, entity) in subsets.iter().enumerate() {
            assert_eq!(app.world().get::<Mesh3d>(*entity).unwrap().0, original[i]);
        }
        app.update();
        assert!(late_ids.iter().all(|id| !app.world().resource::<Assets<Mesh>>().contains(*id)));
    }

    #[test]
    fn repeated_heads_allocate_independent_meshes_and_sample_independent_poses() {
        if !aa_cry::pak_available() { return; }
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
        world.init_resource::<aa_cry::scene::CryCache>();
        let mut state = bevy::ecs::system::SystemState::<CryAssets>::new(&mut world);
        let mut cry = state.get_mut(&mut world).unwrap();
        for (head, cal, sequence) in [
            ("objects/characters/nuian/male/face/nu_m_face01.chr", "objects/characters/nuian/male/nude/nu_m_base.cal", "animations/nuian/male/facial/fist_ac_talk_11.fsq"),
            ("objects/characters/people/npc_chef01/face/npc_chef01_face01.chr", "objects/characters/people/npc_chef01/nude/npc_chef01_base.cal", "animations/people/chef_01/facial/fist_ac_talk_11.fsq"),
            ("objects/characters/people/girl01/face/face01/girl01_face01.chr", "objects/characters/people/girl01/nude/girl01_base.cal", "animations/people/girl01/facial/fist_ac_talk_02.fsq"),
        ] {
            let plain = cry.load_model(head, &aa_cry::scene::ModelOptions::default()).unwrap();
            // Warm source/topology/CAL caches separately from repeated preparation timing.
            let warm = prepare_loaded_face(&mut cry, head, cal, &plain).unwrap();
            let bytes: usize = warm.0.parts.iter().flat_map(|(_, p)| p).map(|p| {
                cry.meshes.get(&p.mesh).unwrap().get_morph_targets().unwrap().len()
                    * std::mem::size_of::<MorphAttributes>()
            }).sum();
            let before = (cry.meshes.len(), cry.images.len(), cry.materials.len(), cry.bindposes.len());
            let timer = std::time::Instant::now();
            let mut instances = (0..16).map(|_| prepare_loaded_face(&mut cry, head, cal, &plain).unwrap()).collect::<Vec<_>>();
            let prepare = timer.elapsed();
            let after = (cry.meshes.len(), cry.images.len(), cry.materials.len(), cry.bindposes.len());
            let subsets = plain.parts.iter().map(|(_, p)| p.len()).sum::<usize>();
            assert_eq!(after, (before.0 + 16 * subsets, before.1, before.2, before.3));
            let ids: HashSet<_> = instances.iter().flat_map(|(_, face, _)| face.meshes.iter().copied()).collect();
            assert_eq!(ids.len(), 16 * subsets);
            let sequence = FacialSequence::load(sequence).unwrap();
            let mut outputs = Vec::new();
            for (i, (_, face, weights)) in instances.iter_mut().enumerate() {
                face.play(sequence.clone(), true);
                face.follow_body = false;
                face.fade_seconds = 0.0;
                face.time = i as f32 / 16.0 * sequence.end;
                let mut output = vec![0.0; face.targets.len()];
                face.update(None, 0.0, &mut output);
                assert!(weights.weights().iter().all(|w| *w == 0.0));
                outputs.push(output);
            }
            assert!(outputs.windows(2).any(|pair| pair[0] != pair[1]));
            let preserved = outputs[1].clone();
            instances[0].1.stop();
            instances[0].1.update(None, 1.0, &mut outputs[0]);
            instances[1].1.update(None, 0.0, &mut outputs[1]);
            assert_eq!(outputs[1], preserved);
            instances[0].1.play(sequence.clone(), true);
            let timer = std::time::Instant::now();
            for _ in 0..600 {
                for ((_, face, _), output) in instances.iter_mut().zip(&mut outputs) {
                    face.update(None, 1.0 / 60.0, output);
                }
            }
            eprintln!("CROWD {head}: targets={}, subsets={subsets}, morph_bytes/instance={bytes}, prepare16={prepare:?}, sample9600={:?}, assets={before:?}->{after:?}", instances[0].1.targets.len(), timer.elapsed());
        }
    }

    #[test]
    fn prepared_meshes_are_released_when_instance_handles_drop() {
        if !aa_cry::pak_available() { return; }
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()));
        app.init_asset::<Mesh>();
        app.init_asset::<Image>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
        app.init_resource::<aa_cry::scene::CryCache>();
        let mut state = bevy::ecs::system::SystemState::<CryAssets>::new(app.world_mut());
        let ids = {
            let mut cry = state.get_mut(app.world_mut()).unwrap();
            let path = "objects/characters/people/npc_chef01/face/npc_chef01_face01.chr";
            let cal = "objects/characters/people/npc_chef01/nude/npc_chef01_base.cal";
            let plain = cry.load_model(path, &aa_cry::scene::ModelOptions::default()).unwrap();
            let prepared = (0..16).map(|_| prepare_loaded_face(&mut cry, path, cal, &plain).unwrap()).collect::<Vec<_>>();
            prepared.iter().flat_map(|(_, face, _)| face.meshes.iter().copied()).collect::<Vec<_>>()
            // All prepared models, their strong handles and animators drop here.
        };
        assert!(ids.iter().all(|id| app.world().resource::<Assets<Mesh>>().contains(*id)));
        app.update();
        assert!(ids.iter().all(|id| !app.world().resource::<Assets<Mesh>>().contains(*id)));
    }

    #[test]
    fn original_npc_loaded_head_catalog() {
        if !aa_cry::pak_available() { return; }
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
        world.init_resource::<aa_cry::scene::CryCache>();
        let mut state = bevy::ecs::system::SystemState::<CryAssets>::new(&mut world);
        let mut cry = state.get_mut(&mut world).unwrap();
        for (head, body, count, prefix, library) in [
            ("npc_chef01/face/npc_chef01_face01", "npc_chef01/nude/npc_chef01_base", 23, "animations/people/chef_01", "animations/pc_expressions.fxl"),
            ("npc_blacksmith01/face/face01/npc_blacksmith01_face01", "npc_blacksmith01/nude/npc_blacksmith01_base", 23, "animations/people/npc_blacksmith01", "animations/pc_expressions.fxl"),
            ("npc_blacksmith01/face/face02/npc_blacksmith01_face02", "npc_blacksmith01/nude/npc_blacksmith01_base", 23, "animations/people/npc_blacksmith01", "animations/pc_expressions.fxl"),
            ("girl01/face/face01/girl01_face01", "girl01/nude/girl01_base", 24, "animations/people/girl01", "animations/people/girl01/facial/girl01.fxl"),
        ] {
            let head = format!("objects/characters/people/{head}.chr");
            let cal = format!("objects/characters/people/{body}.cal");
            let loaded = cry.load_model(&head, &aa_cry::scene::ModelOptions::default()).unwrap();
            let (prepared, mut face, weights) = prepare_loaded_face(&mut cry, &head, &cal, &loaded).unwrap();
            let native = crate::MorphSet::load(&head).unwrap().unwrap();
            assert_eq!(face.targets, native.targets.iter().map(|target| target.name.clone()).collect::<Vec<_>>());
            assert_eq!(face.targets.len(), count);
            assert_eq!(weights.weights().len(), count);
            assert!(Arc::ptr_eq(&face.library, &ExpressionLibrary::load(library).unwrap()));
            for (_, subsets) in &prepared.parts {
                for subset in subsets {
                    assert_eq!(cry.meshes.get(&subset.mesh).unwrap().morph_target_names().unwrap(), &face.targets);
                }
            }
            let table = Cal::shared(&cal).unwrap();
            let name = if count == 24 { "fist_ba_relaxed_idle" } else { "fist_ac_talk_11" };
            let body = table.animation(name).unwrap();
            let authored = table.facial.get(name).unwrap();
            assert!(authored.path.starts_with(prefix), "{}", authored.path);
            assert_eq!(face.sequences.get(&body.path), Some(&authored.path));
            // Exercise the selected original library/FSQ on these actual native targets.
            face.follow_body = false;
            face.fade_seconds = 0.0;
            let sequence = FacialSequence::load(&authored.path).unwrap();
            face.play(sequence, false);
            let mut output = vec![0.0; count];
            let mut peak = 0.0f32;
            for frame in 0..120 {
                face.time = frame as f32 / 30.0;
                face.update(None, 0.0, &mut output);
                peak = peak.max(output.iter().copied().fold(0.0f32, |a, b| a.max(b.abs())));
            }
            assert!(peak > 0.1, "{head}: authored speech remained neutral");
        }
    }

    #[test]
    fn loaded_factory_isolates_meshes_and_follows_original_npc_body_clock() {
        if !aa_cry::pak_available() { return; }
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>();
        world.init_resource::<aa_cry::scene::CryCache>();
        let mut state = bevy::ecs::system::SystemState::<CryAssets>::new(&mut world);
        let mut cry = state.get_mut(&mut world).unwrap();
        let path = "objects/characters/people/npc_chef01/face/npc_chef01_face01.chr";
        let cal_path = "objects/characters/people/npc_chef01/nude/npc_chef01_base.cal";
        let plain = cry.load_model(path, &aa_cry::scene::ModelOptions::default()).unwrap();
        let counts = (cry.meshes.len(), cry.images.len(), cry.materials.len(), cry.bindposes.len());
        let body_path = "objects/characters/people/npc_chef01/nude/npc_chef01_base.chr";
        assert!(prepare_loaded_face(&mut cry, body_path, cal_path, &plain).is_err());
        assert_eq!((cry.meshes.len(), cry.images.len(), cry.materials.len(), cry.bindposes.len()), counts);
        assert!(prepare_loaded_face(&mut cry, path, "missing-body.cal", &plain).is_err());
        assert_eq!((cry.meshes.len(), cry.images.len(), cry.materials.len(), cry.bindposes.len()), counts);
        let mut incomplete = (*plain).clone();
        incomplete.parts[0].1.pop();
        assert!(prepare_loaded_face(&mut cry, path, cal_path, &incomplete).is_err());
        assert_eq!((cry.meshes.len(), cry.images.len(), cry.materials.len(), cry.bindposes.len()), counts);
        let (first, mut face, weights) = prepare_loaded_face(&mut cry, path, cal_path, &plain).unwrap();
        let (second, other_face, _) = prepare_loaded_face(&mut cry, path, cal_path, &plain).unwrap();
        assert_eq!(face.targets.len(), 23);
        assert_eq!(weights.weights(), &[0.0; 23]);
        for (((_, plain), (_, first)), (_, second)) in plain.parts.iter().zip(&first.parts).zip(&second.parts) {
            for ((plain, first), second) in plain.iter().zip(first).zip(second) {
                assert_ne!(plain.mesh, first.mesh);
                assert_ne!(first.mesh, second.mesh);
                assert!(!cry.meshes.get(&plain.mesh).unwrap().has_morph_targets());
                let prepared = cry.meshes.get(&first.mesh).unwrap();
                assert!(prepared.has_morph_targets());
                assert_eq!(prepared.morph_target_names().unwrap(), &face.targets);
                assert!(face.meshes.contains(&first.mesh.id()));
                assert!(!other_face.meshes.contains(&first.mesh.id()));
            }
        }
        let cal = Cal::shared(cal_path).unwrap();
        let talk = cal.animation("fist_ac_talk_11").unwrap();
        let rig = shared_rig("objects/characters/people/npc_chef01/nude/npc_chef01_base.chr").unwrap();
        let bound = ClipLibrary::default().bound(&rig, &talk.path, SampleOptions::default()).unwrap();
        let mut body = Animator::new(rig, Vec::new());
        body.play(bound, false, 0.0);
        body.paused = true;
        body.current.as_mut().unwrap().time = 1.7218;
        face.fade_seconds = 0.0;
        let mut out = vec![0.0; 23];
        face.update(Some(&body), 0.01, &mut out);
        assert_eq!(face.time, 1.7218);
        assert!(out.iter().any(|w| *w > 0.5), "{out:?}");
        let expected = out.clone();
        face.update(Some(&body), 5.0, &mut out);
        assert_eq!(out, expected);
        body.current.as_mut().unwrap().time = 3.0;
        face.update(Some(&body), 0.01, &mut out);
        assert_eq!(face.time, 3.0);
        // The authored return retains small signed values; do not force neutral.
        assert!(out.iter().all(|w| w.abs() < 0.1), "{out:?}");
        assert!(out.iter().any(|w| *w < -0.01), "{out:?}");
    }

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
        // Re-enable the actual head controller, then park the body as the client's
        // Asleep component does. Detach must still undo the last facial offset.
        app.world_mut().get_mut::<FacialAnimator>(root).unwrap().library = ExpressionLibrary::load("animations/pc_expressions.fxl").unwrap();
        step(&mut app, 0.01);
        assert!(app.world().get::<Transform>(entity).unwrap().rotation.angle_between(body_rotation) > 0.1);
        app.world_mut().entity_mut(root).insert(AttachedCharacterFace(Vec::new()));
        let _parked_body = app.world_mut().entity_mut(root).take::<Animator>().unwrap();
        assert!(disable_character_face(app.world_mut(), root));
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
    let geometry = crate::character::face_geometry(cry, assets, part, &loaded.model)?;
    prepare_face_inner(cry, &part.path, &assets.cal, loaded, &geometry)
}

pub(crate) fn prepare_character_face(
    cry: &mut CryAssets, assets: &CharacterAssets, part: &PartAsset, loaded: &LoadedModel,
) -> Result<(LoadedModel, FacialAnimator, MorphWeights)> {
    let geometry = crate::character::face_geometry(cry, assets, part, &loaded.model)?;
    let parts = loaded.parts.iter().map(|(node, parts)| (*node, parts.iter().map(|p| p.mesh.clone()).collect())).collect::<Vec<_>>();
    let (handles, mut face, weights) = prepare_private_subsets(cry, &part.path, &assets.cal, (&loaded.model, &geometry), &parts, None)?;
    let mut isolated = loaded.clone();
    for (_, parts) in &mut isolated.parts {
        for part in parts { part.mesh = handles[&part.mesh.id()].clone(); }
    }
    face.meshes = face.meshes.iter().map(|id| handles[id].id()).collect();
    Ok((isolated, face, weights))
}

/// Prepare an already loaded head before passing it to `aa_cry::scene::spawn_loaded`.
/// Returns an isolated model with GPU morph descriptors, its animator and root weights.
/// Insert the latter two on the entity holding the body `Animator`, and parent the spawned
/// head below that entity. Requires `FacialPlugin`. The input model/cache stays unchanged;
/// each result owns distinct mesh handles so independently animated instances cannot alias.
pub fn prepare_loaded_face(
    cry: &mut CryAssets,
    head_path: &str,
    body_cal_path: &str,
    loaded: &LoadedModel,
) -> Result<(LoadedModel, FacialAnimator, MorphWeights)> {
    let parts = loaded.parts.iter().map(|(node, parts)| (*node, parts.iter().map(|p| p.mesh.clone()).collect())).collect::<Vec<_>>();
    let (handles, mut face, weights) = prepare_private_subsets(cry, head_path, body_cal_path, (&loaded.model, &loaded.model), &parts, None)?;
    let mut isolated = loaded.clone();
    for (_, parts) in &mut isolated.parts {
        for part in parts {
            part.mesh = handles[&part.mesh.id()].clone();
        }
    }
    face.meshes = face.meshes.iter().map(|id| handles[id].id()).collect();
    Ok((isolated, face, weights))
}

fn prepare_private_subsets(
    cry: &mut CryAssets, head_path: &str, body_cal_path: &str,
    models: (&aa_cry::Model, &aa_cry::Model), parts: &[(usize, Vec<Handle<Mesh>>)], rebuilt: Option<&HashMap<AssetId<Mesh>, Mesh>>,
) -> Result<(HashMap<AssetId<Mesh>, Handle<Mesh>>, FacialAnimator, MorphWeights)> {
    // Parse source data and build/validate every descriptor before allocating assets.
    // Unsupported CDF skin parts must leave the plain cache and asset counts untouched.
    let (face, prepared) = prepare_face_data(cry, head_path, body_cal_path, models, parts, rebuilt)?;
    let weights = MorphWeights::new(vec![0.0; face.targets.len()], None).map_err(|e| Error::Format(e.to_string()))?;
    let clones = prepared.into_iter().map(|(handle, attributes)| {
        let mut mesh = rebuilt.and_then(|meshes| meshes.get(&handle.id())).or_else(|| cry.meshes.get(&handle))
            .ok_or_else(|| Error::Missing("facial mesh already extracted".into()))?.clone();
        mesh.try_set_morph_targets(attributes).map_err(|e| Error::Format(e.to_string()))?;
        mesh.set_morph_target_names(face.targets.clone());
        Ok((handle.id(), mesh))
    }).collect::<Result<Vec<_>>>()?;
    let handles: HashMap<_, _> = clones.into_iter().map(|(id, mesh)| (id, cry.meshes.add(mesh))).collect();
    Ok((handles, face, weights))
}

fn prepare_face_inner(cry: &mut CryAssets, head_path: &str, body_cal_path: &str, loaded: &LoadedModel, geometry: &aa_cry::Model) -> Result<FacialAnimator> {
    let parts = loaded.parts.iter().map(|(node, parts)| (*node, parts.iter().map(|p| p.mesh.clone()).collect())).collect::<Vec<_>>();
    let (face, prepared) = prepare_face_data(cry, head_path, body_cal_path, (&loaded.model, geometry), &parts, None)?;
    for (handle, attributes) in prepared {
        let mut mesh = cry.meshes.get_mut(&handle).ok_or_else(|| Error::Missing("facial mesh already extracted".into()))?;
        mesh.try_set_morph_targets(attributes).map_err(|e| Error::Format(e.to_string()))?;
        mesh.set_morph_target_names(face.targets.clone());
    }
    Ok(face)
}

fn prepare_face_data(
    cry: &CryAssets, head_path: &str, body_cal_path: &str, models: (&aa_cry::Model, &aa_cry::Model), parts: &[(usize, Vec<Handle<Mesh>>)], rebuilt: Option<&HashMap<AssetId<Mesh>, Mesh>>,
) -> Result<(FacialAnimator, Vec<(Handle<Mesh>, Vec<MorphAttributes>)>)> {
    let (model, geometry) = models;
    let set = expression_set(head_path, model)?;
    if set.targets.len() > bevy::mesh::morph::MAX_MORPH_WEIGHTS {
        return Err(Error::Format("too many facial targets".into()));
    }
    // The playable heads use unprefixed morphs. The older race-specific libraries contain
    // el_m_*/fe_m_* morphs and close_l controllers that do not match these meshes/current FSQs.
    let library = head_library(head_path)?;
    let cal = Cal::shared(body_cal_path)?;
    let sequences = cal
        .animations
        .iter()
        .filter_map(|(name, body)| cal.facial.get(name).map(|face| (body.path.clone(), face.path.clone())))
        .collect();
    let targets: Vec<_> = set.targets.iter().map(|t| t.name.clone()).collect();
    let mut meshes = Vec::new();
    let mut prepared = Vec::new();
    for (ni, parts) in parts {
        let source = &geometry.meshes[geometry.nodes[*ni]
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
            let mesh = rebuilt.and_then(|meshes| meshes.get(&part.id())).or_else(|| cry.meshes.get(part))
                .ok_or_else(|| Error::Missing("facial mesh already extracted".into()))?;
            if mesh
                .try_attribute(Mesh::ATTRIBUTE_POSITION)
                .map_err(|e| Error::Format(e.to_string()))?
                .len()
                != order.len()
            {
                return Err(Error::Format("facial subset vertex remap differs".into()));
            }
            prepared.push((part.clone(), attributes));
            meshes.push(part.id());
        }
    }
    let count = targets.len();
    let channels = targets.clone();
    Ok((FacialAnimator {
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
    }, prepared))
}
