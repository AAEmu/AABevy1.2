//! NPC and monster models that are not player characters: a `.cdf` (skeleton model plus skin
//! attachments) or a `.chr`, rigged and animated through aa-anim, or a static `.cgf`.

use aa_anim::{Animator, Cdf, ClipLibrary, spawn_bones};
use aa_cry::scene::{CryAssets, ModelOptions, SkeletonBinding, spawn_loaded};
use aa_cry::CRY_TO_BEVY;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion};

/// The original head CAL declares its facial library. This avoids parsing body/clothing
/// geometry as expression targets for every member of a crowd; CAL results are shared.
fn authored_face(path: &str) -> bool {
    let stem = path.rsplit_once('.').map_or(path, |(stem, _)| stem);
    let cal = format!("{stem}.cal");
    aa_pak::Pak::global().exists(&cal) && aa_anim::Cal::shared(&cal).is_ok_and(|cal| cal.face_library.is_some())
}

/// The model files (with their material override) [`spawn`] loads for `file`, to prepare them ahead on a worker thread.
pub fn model_files(file: &str) -> Vec<(String, Option<String>)> {
    let (main, main_material, attachments) = if file.ends_with(".cdf") {
        let Ok(cdf) = Cdf::load(file) else { return Vec::new() };
        let parts = cdf
            .attachments
            .iter()
            .filter(|a| a.kind == aa_anim::cdf::AttachmentKind::Skin && !a.binding.is_empty() && a.flags & 1 == 0)
            .map(|a| (a.binding.clone(), a.material.clone()))
            .collect::<Vec<_>>();
        (cdf.model, cdf.material, parts)
    } else {
        (file.to_string(), None, Vec::new())
    };
    if main.is_empty() {
        return Vec::new();
    }
    let mut out = vec![(main, main_material.clone())];
    for (path, material) in attachments {
        if path.ends_with(".cdf") {
            if let Ok(c) = Cdf::load(&path) {
                out.push((c.model, c.material.or(material)));
            }
        } else {
            out.push((path, material));
        }
    }
    out
}

/// Spawns the model at `file` (`.cdf`, `.chr` or `.cgf`, pak path). Returns the root entity with
/// `transform` (feet at the origin, facing Bevy -Z at identity rotation like game north), or `None`
/// when the file cannot be loaded.
pub fn spawn(commands: &mut Commands, cry: &mut CryAssets, clips: &ClipLibrary, file: &str, transform: Transform) -> Option<Entity> {
    // the base skin and the extra skin attachments
    let (main, main_material, attachments) = if file.ends_with(".cdf") {
        let cdf = match Cdf::load(file) {
            Ok(c) => c,
            Err(e) => {
                debug!("creature {file}: {e}");
                return None;
            }
        };
        let mut parts = Vec::new();
        for a in &cdf.attachments {
            // `Flags = 1` marks an empty slot; weapons on bones (CA_BONE) are not drawn
            if a.kind != aa_anim::cdf::AttachmentKind::Skin || a.binding.is_empty() || a.flags & 1 != 0 {
                continue;
            }
            parts.push((a.binding.clone(), a.material.clone()));
        }
        (cdf.model, cdf.material, parts)
    } else {
        (file.to_string(), None, Vec::new())
    };
    if main.is_empty() {
        return None;
    }

    let rig = aa_anim::pose::shared_rig(&main).ok().filter(|r| !r.is_empty());
    let Some(rig) = rig else {
        // a static model
        let options = ModelOptions::default();
        let loaded = cry.load_model(&main, &options).ok()?;
        let model = spawn_loaded(commands, &main, &loaded, &options);
        let root = commands.spawn((Name::new(main.clone()), transform, Visibility::default())).id();
        commands.entity(model).insert(ChildOf(root));
        return Some(root);
    };
    let skeleton = rig.skeleton.clone();
    let root = commands.spawn((Name::new(main.clone()), transform, Visibility::default())).id();
    let model_root = commands.spawn((Name::new("model"), Transform::from_rotation(CRY_TO_BEVY), Visibility::default(), ChildOf(root))).id();
    let bones = spawn_bones(commands, model_root, &rig);
    let cal_path = format!("{}.cal", main.trim_end_matches(".chr"));
    let mut facial = None;

    let mut paths = vec![(main.clone(), main_material)];
    paths.extend(attachments);
    for (path, material) in paths {
        // attached parts may themselves be a `.cdf` wrapper around a `.chr`
        let (path, material) = if path.ends_with(".cdf") {
            match Cdf::load(&path) {
                Ok(c) => (c.model, c.material.or(material)),
                Err(_) => continue,
            }
        } else {
            (path, material)
        };
        let options = ModelOptions {
            material,
            skeleton: Some(SkeletonBinding { skeleton: skeleton.clone(), joints: bones.clone() }),
            cry_space: true,
            ..default()
        };
        let Ok(loaded) = cry.load_model(&path, &options) else { continue };
        let prepared = if facial.is_none() && authored_face(&path) {
            aa_anim::facial::prepare_loaded_face(cry, &path, &cal_path, &loaded).ok()
        } else { None };
        let part = spawn_loaded(commands, &path, prepared.as_ref().map_or(loaded.as_ref(), |(model, _, _)| model), &options);
        commands.entity(part).insert(ChildOf(model_root));
        if let Some((_, face, weights)) = prepared {
            facial = Some((face, weights));
        }
    }

    // animations: `<model>.cal` next to the skeleton file
    let mut animator = Animator::new(rig.clone(), bones);
    let mut entity = commands.entity(root);
    if let Some(set) = super::anim::shared_clip_set(&main, &cal_path, clips) {
        animator.play(set.idle.clone(), true, 0.0);
        // monsters stand in a fighting stance while they fight (`fist_ba_combat_idle`)
        if let Some(combat) = super::idle::combat_idle(&rig, &set.cal, clips) {
            entity.insert(super::idle::IdleClips::new(None, Some(combat)));
        }
        entity.insert(Locomotion::new((*set).clone()));
    }
    entity.insert((animator, Motion::default()));
    if let Some(facial) = facial { entity.insert(facial); }
    Some(root)
}
