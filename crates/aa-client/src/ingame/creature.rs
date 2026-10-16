//! NPC and monster models that are not player characters: a `.cdf` (skeleton model plus skin
//! attachments) or a `.chr`, rigged and animated through aa-anim, or a static `.cgf`.

use std::sync::Arc;

use aa_anim::pose::Rig;
use aa_anim::{Animator, Cal, Cdf, ClipLibrary, spawn_bones};
use aa_cry::scene::{CryAssets, ModelOptions, SkeletonBinding, spawn_loaded};
use aa_cry::{CRY_TO_BEVY, Skeleton};
use bevy::prelude::*;

use super::anim::{ClipSet, Locomotion, Motion};

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

    let skeleton = Skeleton::load(&main).ok().filter(|s| !s.is_empty());
    let Some(skeleton) = skeleton else {
        // a static model
        let options = ModelOptions::default();
        let loaded = cry.load_model(&main, &options).ok()?;
        let model = spawn_loaded(commands, &main, &loaded, &options);
        let root = commands.spawn((Name::new(main.clone()), transform, Visibility::default())).id();
        commands.entity(model).insert(ChildOf(root));
        return Some(root);
    };
    let skeleton = Arc::new(skeleton);
    let rig = Arc::new(Rig::new(skeleton.clone()));
    let root = commands.spawn((Name::new(main.clone()), transform, Visibility::default())).id();
    let model_root = commands.spawn((Name::new("model"), Transform::from_rotation(CRY_TO_BEVY), Visibility::default(), ChildOf(root))).id();
    let bones = spawn_bones(commands, model_root, &rig);

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
        let part = spawn_loaded(commands, &path, &loaded, &options);
        commands.entity(part).insert(ChildOf(model_root));
    }

    // animations: `<model>.cal` next to the skeleton file
    let cal_path = format!("{}.cal", main.trim_end_matches(".chr"));
    let mut animator = Animator::new(rig.clone(), bones);
    let mut entity = commands.entity(root);
    if let Some(set) = Cal::load(&cal_path).ok().and_then(|cal| ClipSet::load(&rig, &cal, clips)) {
        animator.play(set.idle.clone(), true, 0.0);
        entity.insert(Locomotion::new(set));
    }
    entity.insert((animator, Motion::default()));
    Some(root)
}
