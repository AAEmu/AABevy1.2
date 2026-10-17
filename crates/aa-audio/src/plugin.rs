//! Bevy side: [`AaAudioPlugin`], the [`Sound`] component and the [`SpatialListener`].

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::transform::TransformSystems;

use crate::engine::{Engine, PlayOptions, Settings, Voice, Volumes};

/// Plays an event from this entity: 3D at its [`GlobalTransform`] when the event is 3D (the sound
/// follows the entity), otherwise 2D. The sound is stopped, with the event's fade out, when the
/// component is removed or the entity despawned.
///
/// When a one-shot event ends the component is removed, or the entity despawned with
/// [`despawn_when_done`](Sound::despawn_when_done); a looping event plays until then.
#[derive(Component, Clone, Debug)]
pub struct Sound {
    /// Event path, e.g. `sounds/x2skills:wild:fire_arrow_form` (see [`crate::library`]).
    pub event: String,
    /// Multiplies the event's own volume.
    pub volume: f32,
    /// Event parameters by name (`daylight`, `speed`, `gender` ...); changing a value of a playing
    /// sound updates it in place.
    pub params: Vec<(String, f32)>,
    pub despawn_when_done: bool,
}

impl Sound {
    pub fn event(path: impl Into<String>) -> Sound {
        Sound { event: path.into(), volume: 1.0, params: Vec::new(), despawn_when_done: false }
    }

    pub fn with_param(mut self, name: &str, value: f32) -> Sound {
        match self.params.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some(p) => p.1 = value,
            None => self.params.push((name.to_string(), value)),
        }
        self
    }

    pub fn with_volume(mut self, volume: f32) -> Sound {
        self.volume = volume;
        self
    }

    pub fn despawn_when_done(mut self) -> Sound {
        self.despawn_when_done = true;
        self
    }
}

/// Marks the entity (the camera) whose [`GlobalTransform`] is where the player listens from.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct SpatialListener;

/// What was last sent to the engine for an entity's sound.
struct Playing {
    voice: Voice,
    event: String,
    volume: f32,
    params: Vec<(String, f32)>,
}

#[derive(Default)]
struct Live {
    by_entity: HashMap<Entity, Playing>,
    by_voice: HashMap<Voice, Entity>,
}

#[derive(Resource, Default)]
struct LiveVoices(Live);

/// Adds the audio engine ([`Engine`], also a resource), the [`Volumes`] resource and the systems that
/// drive [`Sound`] and [`SpatialListener`]. Silent when the run is unattended (see
/// [`crate::audible_by_default`]).
pub struct AaAudioPlugin {
    pub settings: Settings,
}

impl Default for AaAudioPlugin {
    fn default() -> Self {
        AaAudioPlugin { settings: Settings::auto() }
    }
}

impl AaAudioPlugin {
    /// The plugin with explicit settings (e.g. `Settings { audible: true, ..Default::default() }`).
    pub fn with(settings: Settings) -> AaAudioPlugin {
        AaAudioPlugin { settings }
    }
}

impl Plugin for AaAudioPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<Engine>() {
            app.insert_resource(Engine::start(self.settings.clone()));
        }
        app.init_resource::<Volumes>().init_resource::<LiveVoices>().add_systems(
            PostUpdate,
            (sync_volumes, sync_listener, reap_finished, stop_removed, start_sounds, follow_entities).chain().after(TransformSystems::Propagate),
        );
    }
}

fn sync_volumes(volumes: Res<Volumes>, engine: Res<Engine>) {
    if volumes.is_changed() {
        engine.set_volumes(*volumes);
    }
}

fn sync_listener(listener: Query<&GlobalTransform, With<SpatialListener>>, engine: Res<Engine>, mut last: Local<Option<(Vec3, Quat)>>) {
    let Some(gt) = listener.iter().next() else { return };
    let (_, rotation, translation) = gt.to_scale_rotation_translation();
    if *last != Some((translation, rotation)) {
        *last = Some((translation, rotation));
        engine.set_listener(translation, rotation);
    }
}

fn reap_finished(mut commands: Commands, engine: Res<Engine>, mut live: ResMut<LiveVoices>, sounds: Query<&Sound>) {
    for voice in engine.take_finished() {
        let Some(entity) = live.0.by_voice.remove(&voice) else { continue };
        live.0.by_entity.remove(&entity);
        let despawn = sounds.get(entity).is_ok_and(|s| s.despawn_when_done);
        if let Ok(mut e) = commands.get_entity(entity) {
            if despawn {
                e.despawn();
            } else {
                e.remove::<Sound>();
            }
        }
    }
}

fn stop_removed(mut removed: RemovedComponents<Sound>, engine: Res<Engine>, mut live: ResMut<LiveVoices>) {
    for entity in removed.read() {
        if let Some(p) = live.0.by_entity.remove(&entity) {
            live.0.by_voice.remove(&p.voice);
            engine.stop(p.voice, true);
        }
    }
}

fn start_sounds(sounds: Query<(Entity, Ref<Sound>, &GlobalTransform)>, engine: Res<Engine>, mut live: ResMut<LiveVoices>) {
    for (entity, sound, gt) in &sounds {
        if let Some(playing) = live.0.by_entity.get_mut(&entity) {
            if !sound.is_changed() || sound.is_added() {
                continue;
            }
            if playing.event.eq_ignore_ascii_case(&sound.event) {
                // the same event: update what changed in place
                for (name, value) in &sound.params {
                    if playing.params.iter().find(|(n, _)| n == name).is_none_or(|(_, v)| v != value) {
                        engine.set_param(playing.voice, name, *value);
                    }
                }
                if playing.volume != sound.volume {
                    engine.set_voice_volume(playing.voice, sound.volume);
                }
                playing.params = sound.params.clone();
                playing.volume = sound.volume;
                continue;
            }
            // another event: play it instead
            let old = live.0.by_entity.remove(&entity).unwrap();
            live.0.by_voice.remove(&old.voice);
            engine.stop(old.voice, true);
        }
        let opts = PlayOptions { volume: sound.volume, category: None, params: sound.params.clone() };
        let voice = engine.play_with(&sound.event, Some(gt.translation()), opts);
        live.0.by_entity.insert(entity, Playing { voice, event: sound.event.clone(), volume: sound.volume, params: sound.params.clone() });
        live.0.by_voice.insert(voice, entity);
    }
}

type MovedSounds<'w, 's> = Query<'w, 's, (Entity, &'static GlobalTransform), (With<Sound>, Changed<GlobalTransform>)>;

fn follow_entities(moved: MovedSounds, engine: Res<Engine>, live: Res<LiveVoices>) {
    for (entity, gt) in &moved {
        if let Some(p) = live.0.by_entity.get(&entity) {
            engine.set_position(p.voice, gt.translation());
        }
    }
}
