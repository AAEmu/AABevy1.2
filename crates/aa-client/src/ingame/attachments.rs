//! Original CDF object/effect payloads on existing actor bones.
use aa_anim::cdf::{Attachment, AttachmentKind, BindingKind};
use aa_anim::{Animator, Character};
use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use bevy::ecs::query::QueryFilter;
use bevy::prelude::*;

#[derive(Component)]
struct AttachedEffect {
    owner: Entity,
    binding: String,
    emitter: Option<Entity>,
}

#[derive(Component)]
struct AttachedObject(Entity);
#[derive(Component)]
struct PartPayloads;

pub(crate) fn model(attachment: &Attachment) -> Option<(String, Option<String>)> {
    if attachment.kind != AttachmentKind::Bone || attachment.binding_kind != BindingKind::Object || attachment.binding.is_empty() || attachment.flags & 1 != 0 {
        return None;
    }
    if attachment.binding.ends_with(".cdf") {
        let cdf = aa_anim::Cdf::load(&attachment.binding).ok()?;
        Some((cdf.model, cdf.material.or_else(|| attachment.material.clone())))
    } else {
        Some((attachment.binding.clone(), attachment.material.clone()))
    }
}

pub(crate) fn spawn(
    commands: &mut Commands,
    cry: &mut CryAssets,
    owner: Entity,
    attachment: &Attachment,
    skeleton: &aa_cry::Skeleton,
    bones: &[Entity],
) -> Option<Entity> {
    if attachment.binding.is_empty() || attachment.flags & 1 != 0 {
        return None;
    }
    let (joint, local) = attachment.bone_binding(skeleton).ok()?;
    let parent = *bones.get(joint)?;
    let transform = Transform::from_matrix(Mat4::from(local));
    match attachment.binding_kind {
        BindingKind::Effect => Some(
            commands
                .spawn((
                    Name::new(format!("CDF effect {}", attachment.name)),
                    AttachedEffect {
                        owner,
                        binding: attachment.binding.clone(),
                        emitter: None,
                    },
                    transform,
                    Visibility::default(),
                    ChildOf(parent),
                ))
                .id(),
        ),
        BindingKind::Object => {
            let (path, material) = model(attachment)?;
            let options = ModelOptions {
                material,
                cry_space: true,
                ..default()
            };
            let loaded = cry.load_model(&path, &options).ok()?;
            let entity = spawn_loaded(commands, &path, &loaded, &options);
            commands.entity(entity).insert((transform, AttachedObject(owner), ChildOf(parent)));
            Some(entity)
        }
        BindingKind::Entity => None,
    }
}

fn character_parts(
    mut commands: Commands,
    mut cry: CryAssets,
    characters: Query<(&Character, Option<&Animator>, Option<&super::units::Asleep>), Changed<Character>>,
    prepared: Query<(), With<PartPayloads>>,
) {
    for (character, animator, asleep) in &characters {
        let Some(animator) = animator.or_else(|| asleep.map(super::units::Asleep::animator)) else {
            continue;
        };
        for (index, part) in character.assets.parts.iter().enumerate() {
            let Some(&owner) = character.parts.get(index) else { continue };
            if prepared.contains(owner) { continue; }
            for attachment in &part.attachments {
                spawn(&mut commands, &mut cry, owner, attachment, &animator.rig.skeleton, &animator.bones);
            }
            commands.entity(owner).insert(PartPayloads);
        }
    }
}

/// Read current local visibility, including ancestor changes made by this frame's cull.
fn visible<F: QueryFilter>(owner: Entity, visibility: &Query<(&Visibility, Option<&ChildOf>), F>) -> Option<bool> {
    let mut current = owner;
    for _ in 0..256 {
        let (value, parent) = visibility.get(current).ok()?;
        match value {
            Visibility::Hidden => return Some(false),
            Visibility::Visible => return Some(true),
            Visibility::Inherited => match parent {
                Some(parent) => current = parent.parent(),
                None => return Some(true),
            },
        }
    }
    None
}

fn effects(mut commands: Commands, visibility: Query<(&Visibility, Option<&ChildOf>)>, mut effects: Query<(Entity, &mut AttachedEffect)>) {
    for (anchor, mut effect) in &mut effects {
        let Some(show) = visible(effect.owner, &visibility) else {
            commands.entity(anchor).despawn();
            continue;
        };
        if !show {
            if let Some(emitter) = effect.emitter.take() {
                commands.entity(emitter).despawn();
            }
        } else if effect.emitter.is_none() {
            effect.emitter = Some(
                commands
                    .spawn((
                        aa_fx::Effect {
                            prime: false,
                            looping: false,
                            despawn_when_finished: false,
                            ..aa_fx::Effect::new(effect.binding.clone())
                        },
                        ChildOf(anchor),
                    ))
                    .id(),
            );
        }
    }
}

fn objects(
    mut commands: Commands,
    owners: Query<(&Visibility, Option<&ChildOf>), Without<AttachedObject>>,
    mut objects: Query<(Entity, &AttachedObject, &mut Visibility)>,
) {
    for (entity, object, mut value) in &mut objects {
        let Some(show) = visible(object.0, &owners) else {
            commands.entity(entity).despawn();
            continue;
        };
        let wanted = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *value != wanted {
            *value = wanted;
        }
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(Update, (character_parts, objects, effects).chain().after(super::units::cull_units).after(super::gear_refresh::refresh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_emitters_follow_part_visibility_and_owner_lifetime() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_systems(Update, (objects, effects).chain());
        let actor = app.world_mut().spawn(Visibility::Inherited).id();
        let part = app.world_mut().spawn((Visibility::Inherited, ChildOf(actor))).id();
        let bone = app.world_mut().spawn((Visibility::Inherited, ChildOf(actor))).id();
        let anchor = app
            .world_mut()
            .spawn((
                AttachedEffect {
                    owner: part,
                    binding: "original.effect.name".into(),
                    emitter: None,
                },
                Visibility::Inherited,
                ChildOf(bone),
            ))
            .id();
        app.update();
        let first = app.world().get::<AttachedEffect>(anchor).unwrap().emitter.unwrap();
        let effect = app.world().get::<aa_fx::Effect>(first).unwrap();
        assert!(!effect.prime && !effect.looping && !effect.despawn_when_finished);
        assert_eq!(effect.scale, 1.0);
        *app.world_mut().get_mut::<Visibility>(actor).unwrap() = Visibility::Hidden;
        app.update();
        assert!(app.world().get_entity(first).is_err());
        assert!(app.world().get::<AttachedEffect>(anchor).unwrap().emitter.is_none());
        *app.world_mut().get_mut::<Visibility>(actor).unwrap() = Visibility::Inherited;
        app.update();
        let second = app.world().get::<AttachedEffect>(anchor).unwrap().emitter.unwrap();
        assert_ne!(first, second, "unhide recreates a native lazy emitter");
        *app.world_mut().get_mut::<Visibility>(part).unwrap() = Visibility::Hidden;
        app.update();
        assert!(
            app.world().get_entity(second).is_err(),
            "part hidden independently of actor must release its effect"
        );
        app.world_mut().despawn(part);
        app.update();
        assert!(app.world().get_entity(anchor).is_err(), "bone-parented anchor cannot outlive its source part");
        app.world_mut().despawn(actor);
        app.update();
        assert_eq!(app.world_mut().query::<&aa_fx::Effect>().iter(app.world()).count(), 0);
    }

    #[test]
    fn original_horse_effect_ids_never_enter_model_prewarm() {
        if !aa_cry::pak_available() {
            return;
        }
        let file = "objects/characters/animals/horse/horse_lord.cdf";
        let cdf = aa_anim::Cdf::load(file).unwrap();
        let effects = cdf.attachments.iter().filter(|a| a.binding_kind == BindingKind::Effect).collect::<Vec<_>>();
        assert_eq!(effects.len(), 4);
        let models = super::super::creature::model_files(file, Some(&cdf));
        assert!(
            effects
                .iter()
                .all(|effect| model(effect).is_none() && !models.iter().any(|m| m.0 == effect.binding))
        );
        let skeleton = aa_cry::Skeleton::load(&cdf.model).unwrap();
        assert!(effects.iter().all(|effect| effect.bone_binding(&skeleton).is_ok()));
    }
}
