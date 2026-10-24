use std::collections::HashSet;

use bevy::ecs::system::SystemState;
use bevy::mesh::skinning::SkinnedMesh;

use super::*;
use crate::Rig;

/// Replace a complete received appearance on an existing ordinary character.
///
/// The caller supplies the unchanged rig/bones from its awake or parked Animator.
/// This function never removes, updates or reinserts Animator or RopeSim. It accepts
/// only the same model, race, gender, skeleton and CAL, with all skin joints mapped
/// to the existing base bones. Actor-model changes need a separate caller policy.
///
/// All fallible work precedes entity changes. On error the previous character,
/// facial playback and model transform remain intact; asset preparation may warm
/// shared caches. Successful replacement owns all new part trees. Receipt authority,
/// stale-job rejection, CDF payloads and carried weapons remain caller-owned.
/// Call before animation/transform propagation, after accepting the current receipt.
pub fn replace_character_parts(
    world: &mut World,
    root: Entity,
    rig: &Rig,
    bones: &[Entity],
    mut assets: CharacterAssets,
) -> Result<()> {
    let old = world
        .get::<Character>(root)
        .ok_or_else(|| Error::Missing("existing Character".into()))?
        .clone();
    validate_actor(world, root, &old, rig, bones, &assets)?;
    let facial = crate::facial::character_face_enabled(world, root)?;
    // Existing seat/clip consumers compare these strings directly.
    assets.skeleton.clone_from(&old.assets.skeleton);
    assets.cal.clone_from(&old.assets.cal);
    let mut state = SystemState::<CryAssets>::new(world);
    let (models, replacement) = {
        let mut cry = state
            .get_mut(world)
            .map_err(|e| Error::Format(e.to_string()))?;
        let cuts = cut_lists(&mut cry, &assets)?;
        let mut models = Vec::with_capacity(assets.parts.len());
        let mut replacement = None;
        for (part, cut) in assets.parts.iter().zip(&cuts) {
            let options = ModelOptions {
                material: part.material.clone(),
                skeleton: Some(SkeletonBinding {
                    skeleton: rig.skeleton.clone(),
                    joints: bones.to_vec(),
                }),
                cry_space: true,
                shading: Shading::Full,
                ..default()
            };
            validate_part_source(&part.path, rig)?;
            let loaded = load_part(&mut cry, &assets, part, &options, cut.as_ref())?;
            if loaded
                .skeleton
                .as_ref()
                .is_some_and(|s| s.map_to(&rig.skeleton).iter().any(Option::is_none))
            {
                return Err(Error::Format(format!(
                    "part {} has joints outside the existing base rig",
                    part.path
                )));
            }
            if loaded.parts.is_empty() {
                return Err(Error::Missing(format!(
                    "renderable character part {}",
                    part.path
                )));
            }
            let loaded = if facial && part.kind == PartKind::Face {
                if replacement.is_some()
                    || loaded.parts.iter().flat_map(|(_, p)| p).any(|p| !p.skinned)
                {
                    return Err(Error::Format(
                        "facial replacement requires one skinned head".into(),
                    ));
                }
                let originals = loaded
                    .parts
                    .iter()
                    .flat_map(|(_, p)| p.iter().map(|p| p.mesh.clone()))
                    .collect::<Vec<_>>();
                let (isolated, face, _) =
                    crate::facial::prepare_character_face(&mut cry, &assets, part, &loaded)?;
                replacement = Some((models.len(), face, originals));
                Arc::new(isolated)
            } else {
                loaded
            };
            models.push((options, loaded));
        }
        (models, replacement)
    };
    if facial {
        let (_, face, _) = replacement
            .as_ref()
            .ok_or_else(|| Error::Missing("replacement face".into()))?;
        crate::facial::validate_replacement_face(world, root, rig.id, face)?;
    }

    // No fallible preparation or actor state changes below this point. The queue
    // is local: errors above cannot leak a partial hierarchy into caller Commands.
    let mut queue = bevy::ecs::world::CommandQueue::default();
    let mut commands = Commands::new(&mut queue, world);
    let parts = assets
        .parts
        .iter()
        .zip(&models)
        .map(|(part, (options, loaded))| {
            let entity = spawn_loaded(&mut commands, &part.path, loaded, options);
            commands.entity(entity).insert(ChildOf(old.model_root));
            entity
        })
        .collect::<Vec<_>>();
    queue.apply(world);
    if let Some((index, face, originals)) = replacement {
        crate::facial::rebind_character_face(world, root, parts[index], face, originals);
    }
    world
        .get_mut::<Transform>(old.model_root)
        .unwrap()
        .translation
        .y += assets.heel - old.assets.heel;
    for part in old.parts {
        world.despawn(part);
    }
    let mut character = world.get_mut::<Character>(root).unwrap();
    character.assets = Arc::new(assets);
    character.parts = parts;
    Ok(())
}

fn validate_part_source(path: &str, rig: &Rig) -> Result<()> {
    let model = aa_cry::Model::load(path)?;
    if let Some(skeleton) = &model.skeleton
        && skeleton.map_to(&rig.skeleton).iter().any(Option::is_none)
    {
        return Err(Error::Format(format!(
            "part {path} has joints outside the existing base rig"
        )));
    }
    Ok(())
}

fn validate_actor(
    world: &World,
    root: Entity,
    old: &Character,
    rig: &Rig,
    bones: &[Entity],
    next: &CharacterAssets,
) -> Result<()> {
    let previous = &old.assets;
    if previous.model_id != next.model_id
        || previous.desc.race != next.desc.race
        || previous.desc.gender != next.desc.gender
        || norm(&previous.skeleton) != norm(&next.skeleton)
        || norm(&previous.cal) != norm(&next.cal)
        || crate::pose::shared_rig(&previous.skeleton)?.id != rig.id
    {
        return Err(Error::Format(
            "incompatible character model, race, gender, skeleton or CAL".into(),
        ));
    }
    if bones.len() != rig.len()
        || bones.iter().copied().collect::<HashSet<_>>().len() != bones.len()
    {
        return Err(Error::Format("existing base bone indexing differs".into()));
    }
    if let Some(animator) = world.get::<Animator>(root)
        && (animator.rig.id != rig.id || animator.bones != bones)
    {
        return Err(Error::Format(
            "supplied bones differ from actor Animator".into(),
        ));
    }
    if old.model_root == root
        || world.get::<ChildOf>(old.model_root).map(ChildOf::parent) != Some(root)
        || world
            .get::<Transform>(old.model_root)
            .is_none_or(|t| !t.translation.is_finite())
        || !previous.heel.is_finite()
        || !next.heel.is_finite()
    {
        return Err(Error::Format("invalid existing model root or heel".into()));
    }
    for (i, &bone) in bones.iter().enumerate() {
        let parent = rig.skeleton.bones[i]
            .parent
            .map_or(old.model_root, |p| bones[p]);
        if world.get::<Transform>(bone).is_none()
            || world.get::<ChildOf>(bone).map(ChildOf::parent) != Some(parent)
            || world
                .get::<Name>(bone)
                .is_none_or(|name| name.as_str() != rig.skeleton.bones[i].name)
        {
            return Err(Error::Format("existing base bone hierarchy differs".into()));
        }
    }
    let base = bones.iter().copied().collect::<HashSet<_>>();
    if old.parts.len() != previous.parts.len()
        || old.parts.iter().copied().collect::<HashSet<_>>().len() != old.parts.len()
    {
        return Err(Error::Format(
            "existing character part indexing differs".into(),
        ));
    }
    for (&entity, part) in old.parts.iter().zip(&previous.parts) {
        if base.contains(&entity)
            || world.get::<ChildOf>(entity).map(ChildOf::parent) != Some(old.model_root)
            || world
                .get::<aa_cry::scene::CryModel>(entity)
                .is_none_or(|m| norm(&m.path) != norm(&part.path))
        {
            return Err(Error::Format(
                "existing character part ownership differs".into(),
            ));
        }
        let mut pending = vec![entity];
        while let Some(entity) = pending.pop() {
            if base.contains(&entity) {
                return Err(Error::Format("part owns a shared base bone".into()));
            }
            if world
                .get::<SkinnedMesh>(entity)
                .is_some_and(|s| s.joints.iter().any(|j| !base.contains(j)))
            {
                return Err(Error::Format(
                    "existing part has joints outside the base rig".into(),
                ));
            }
            if let Some(children) = world.get::<Children>(entity) {
                pending.extend(children.iter());
            }
        }
    }
    Ok(())
}
