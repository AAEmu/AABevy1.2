//! The Bevy plugin with a muted engine: no pak and no device needed.

use std::time::{Duration, Instant};

use aa_audio::{AaAudioPlugin, Category, Engine, PcmClip, PcmOptions, Settings, Sound, SpatialListener, Volumes};
use bevy::prelude::*;
use bevy::transform::TransformPlugin;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, AaAudioPlugin::with(Settings { audible: false, ..Settings::default() })));
    app
}

/// The engine handles messages on its own thread: wait until `check` holds.
fn eventually(app: &App, check: impl Fn(&Engine) -> bool) -> bool {
    let engine = app.world().resource::<Engine>();
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if check(engine) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn sound_components_reach_the_engine_once() {
    let mut app = app();
    app.world_mut().spawn((Transform::from_xyz(0.0, 1.7, 0.0), SpatialListener));
    let emitter = app.world_mut().spawn((Transform::from_xyz(5.0, 0.0, -3.0), Sound::event("sounds/x2skills:wild:fire_arrow_form"))).id();
    app.update();
    app.update();
    assert!(eventually(&app, |e| e.stats().requested == 1), "{:?}", app.world().resource::<Engine>().stats());
    assert!(!app.world().resource::<Engine>().stats().audible);

    // moving the entity does not start it again
    app.world_mut().entity_mut(emitter).get_mut::<Transform>().unwrap().translation.x = 6.0;
    app.update();
    app.update();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(app.world().resource::<Engine>().stats().requested, 1);

    // replacing the component plays the new event
    app.world_mut().entity_mut(emitter).insert(Sound::event("sounds/x2interface:interface:button_default"));
    app.update();
    assert!(eventually(&app, |e| e.stats().requested == 2));

    // removing it and despawning the entity are fine
    app.world_mut().entity_mut(emitter).remove::<Sound>();
    app.update();
    app.world_mut().entity_mut(emitter).despawn();
    app.update();
}

#[test]
fn volumes_resource_is_inserted_and_engine_is_shared() {
    let mut app = app();
    app.update();
    assert_eq!(*app.world().resource::<Volumes>(), Volumes::default());
    app.world_mut().resource_mut::<Volumes>().music = 0.25;
    app.update();
    let engine = app.world().resource::<Engine>().clone();
    // play_2d works from anywhere that holds the engine
    engine.play_2d("sounds/x2interface:login_stage:button_login");
    assert!(eventually(&app, |e| e.stats().requested == 1));
}

#[test]
fn tracked_pcm_observers_survive_the_plugins_global_completion_drain() {
    let mut app = app();
    let engine = app.world().resource::<Engine>().clone();
    let clip = PcmClip::mono(24_000, &[0.1]).unwrap();
    let (id, observer) = engine.play_pcm_tracked(&clip, PcmOptions::new(Category::UserMusic)).unwrap();
    let clone = observer.clone();
    assert!(engine.wait_idle(Duration::from_secs(5)));
    assert!(observer.is_finished() && !observer.was_started());
    app.update(); // AaAudioPlugin consumes take_finished(), including this external PCM voice.
    assert!(engine.take_finished().is_empty());
    assert!(!engine.is_playing(id));
    drop(engine);
    drop(app);
    drop(clip);
    assert!(clone.is_finished() && !clone.was_started());
}

#[test]
#[ignore = "needs the game pak; opens no audio device"]
fn failed_sounds_are_reaped_and_despawn_when_requested() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, AaAudioPlugin::with(Settings { audible: true, max_voices: 0, ..Settings::default() })));
    let missing = app.world_mut().spawn((Transform::default(), Sound::event("sounds/x2voice_prologue:system:6170"))).id();
    let refused = app.world_mut().spawn((Transform::default(), Sound::event("sounds/x2interface:interface:button_default").despawn_when_done())).id();
    app.update();
    assert!(app.world().resource::<Engine>().wait_idle(Duration::from_secs(5)));
    app.update();
    assert!(app.world().get::<Sound>(missing).is_none());
    assert!(app.world().get_entity(refused).is_err());
    let stats = app.world().resource::<Engine>().stats();
    assert_eq!((stats.unknown_events, stats.refused), (1, 1));
    assert_eq!(stats.started, 0);
}

#[test]
fn unattended_runs_are_muted_by_default() {
    // the test binary has no --shot, --exit-after, --mute or --sound
    if std::env::var("AA_MUTE").is_ok_and(|v| v == "1") {
        assert!(!aa_audio::audible_by_default());
    } else {
        assert!(aa_audio::audible_by_default());
    }
}
