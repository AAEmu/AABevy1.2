//! Allocate private facial morphs only for the selected, visible, awake remote character.
//! The local player's existing spawn-time face remains independent.
use super::{Asleep, RemoteUnit};
use aa_anim::{Animator, Character};
use aa_net::state::Target;
use bevy::prelude::*;

pub(super) fn select(
    mut commands: Commands,
    target: Res<Target>,
    characters: Query<(Entity, &RemoteUnit, &Visibility), (With<Character>, With<Animator>, Without<Asleep>)>,
    mut selected: Local<Option<Entity>>,
) {
    let wanted = target.obj_id.and_then(|object| {
        characters
            .iter()
            .find(|(_, unit, visibility)| unit.obj_id == object && **visibility != Visibility::Hidden)
            .map(|(entity, _, _)| entity)
    });
    if *selected == wanted {
        return;
    }
    let previous = std::mem::replace(&mut *selected, wanted);
    commands.queue(move |world: &mut World| {
        if let Some(entity) = previous {
            aa_anim::facial::disable_character_face(world, entity);
        }
        if let Some(entity) = wanted {
            if world.get::<Animator>(entity).is_some()
                && world.get::<Character>(entity).is_some()
                && world.get::<Asleep>(entity).is_none()
                && world.get::<Visibility>(entity).is_some_and(|v| *v != Visibility::Hidden)
            {
                if let Err(error) = aa_anim::facial::enable_character_face(world, entity) {
                    debug!("unit {entity:?} selected face unavailable: {error}");
                }
            }
        }
    });
}
