//! PCM uses the real engine/device at master zero; no WAV, FEV or pak needed except the last test.
//! Device-dependent tests explicitly report a skip if the engine cannot open the device.

use std::time::{Duration, Instant};

use aa_audio::{Category, Engine, Library, PcmClip, PcmError, PcmOptions, PcmSpatial, Settings, Voice, Volumes};
use bevy::math::{Quat, Vec3};

fn until(mut check: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn engine(max_voices: usize) -> Engine {
    let engine = Engine::start(Settings { audible: true, max_voices, ..Settings::default() });
    engine.set_volumes(Volumes { master: 0.0, ..Volumes::default() });
    engine
}

fn device_ready(engine: &Engine, first: Voice) -> bool {
    assert!(until(|| engine.stats().started > 0 || engine.stats().device_error.is_some()), "{:?}", engine.stats());
    if let Some(error) = engine.stats().device_error {
        assert!(engine.wait_idle(Duration::from_secs(5)));
        assert!(!engine.is_playing(first));
        assert_eq!(engine.take_finished(), [first]);
        assert!(engine.take_finished().is_empty());
        assert_eq!(engine.stats().pcm_submissions, 0);
        eprintln!("SKIPPED device-backed PCM playback: {error}");
        return false;
    }
    true
}

fn assert_finished_once(engine: &Engine, mut expected: Vec<Voice>) {
    assert!(engine.wait_idle(Duration::from_secs(5)), "{:?}", engine.stats());
    assert!(expected.iter().all(|&v| !engine.is_playing(v)));
    let mut finished = engine.take_finished();
    expected.sort_by_key(|v| v.0);
    finished.sort_by_key(|v| v.0);
    assert_eq!(finished, expected);
    assert!(engine.take_finished().is_empty());
    assert_eq!(engine.stats().pcm_submissions, 0);
}

#[test]
fn pcm_muted_and_refused_requests_complete_without_a_device() {
    let clip = PcmClip::mono(24_000, &[0.1; 240]).unwrap();
    for (audible, max_voices) in [(false, 64), (true, 0)] {
        let engine = Engine::start(Settings { audible, max_voices, ..Settings::default() });
        for _ in 0..16 {
            let id = engine.play_pcm(&clip, PcmOptions::new(Category::Music)).unwrap();
            if !audible {
                assert!(!engine.is_playing(id));
            }
            engine.stop(id, false);
            assert_finished_once(&engine, vec![id]);
        }
        assert!(engine.stats().device_error.is_none());
        assert_eq!(engine.stats().started, 0);
        assert_eq!(engine.stats().decoded_sounds, 0);
    }
}

#[test]
#[ignore = "opens an audio device at master volume zero"]
fn pcm_tracked_device_admission_and_completion_survive_global_drains() {
    let engine = engine(64);
    engine.set_volumes(Volumes { master: 0.0, user_music: 0.0, ..Volumes::default() });
    let clip = PcmClip::mono(24_000, &[0.1; 4_800]).unwrap();
    let opts = PcmOptions::new(Category::UserMusic);
    let (first, observer) = engine.play_pcm_tracked(&clip, opts).unwrap();
    if !device_ready(&engine, first) {
        assert!(observer.is_finished() && !observer.was_started());
        return;
    }
    assert!(until(|| observer.is_finished()), "{:?}", engine.stats());
    assert!(observer.was_started(), "master/category zero still accepts a real Kira handle");
    assert_finished_once(&engine, vec![first]);
    assert!(observer.clone().is_finished() && observer.was_started());

    let tiny = PcmClip::mono(24_000, &[0.1]).unwrap();
    let (tiny_id, tiny_observer) = engine.play_pcm_tracked(&tiny, opts).unwrap();
    assert_finished_once(&engine, vec![tiny_id]);
    assert!(tiny_observer.is_finished() && tiny_observer.was_started());

    let long = PcmClip::mono(24_000, &[0.1; 24_000]).unwrap();
    let (stopped, stop_observer) = engine.play_pcm_tracked(&long, opts).unwrap();
    assert!(until(|| stop_observer.was_started()));
    engine.stop(stopped, true);
    engine.stop_all();
    assert_finished_once(&engine, vec![stopped]);
    assert!(stop_observer.is_finished() && stop_observer.was_started());
    assert_eq!((engine.stats().started, engine.stats().music_ended), (3, 0));
    assert_eq!((engine.stats().cache_bytes, engine.stats().decoded_sounds), (0, 0));
    eprintln!("Tracked PCM real-device lifecycle: {:?}", engine.stats());
    drop(engine);
    assert!(observer.is_finished() && observer.was_started());
}

#[test]
#[ignore = "opens an audio device at master volume zero"]
fn pcm_natural_completion_and_runtime_controls_release_quota() {
    let engine = engine(64);
    let short = PcmClip::mono(24_000, &vec![0.1; 4_800]).unwrap();
    let first = engine.play_pcm(&short, PcmOptions::new(Category::Music)).unwrap();
    if !device_ready(&engine, first) {
        return;
    }
    assert_finished_once(&engine, vec![first]);
    assert_eq!(engine.stats().music_ended, 0, "PCM is a voice, not the background-music slot");
    assert!(engine.music_playing().is_none());

    let mut opts = PcmOptions::new(Category::Voice);
    // Starts out of range; this must still finish, without a permanent culled/paused voice.
    opts.spatial = Some(PcmSpatial { position: Vec3::splat(1_000.0), min_distance: 1.0, max_distance: 30.0 });
    let spatial = engine.play_pcm(&short, opts).unwrap();
    engine.set_position(spatial, Vec3::NAN); // invalid updates are ignored
    engine.set_position(spatial, Vec3::X * 2.0);
    engine.set_listener(Vec3::ZERO, Quat::IDENTITY);
    engine.set_voice_volume(spatial, f32::NAN);
    engine.set_voice_volume(spatial, 0.0);
    engine.set_param(spatial, "time_loop", 99.0); // PCM has no event parameters
    engine.set_volumes(Volumes { master: 0.0, voice: 0.0, ..Volumes::default() });
    assert_finished_once(&engine, vec![spatial]);
    assert_eq!(engine.stats().started, 2);
    assert_eq!(engine.stats().cache_bytes, 0);
    assert_eq!(engine.stats().decoded_sounds, 0);
    eprintln!("PCM natural/runtime lifecycle: {:?}", engine.stats());
}

#[test]
#[ignore = "opens an audio device at master volume zero"]
fn pcm_user_music_track_runs_silent_muted_and_stopped_voices_to_one_completion() {
    let engine = engine(64);
    engine.set_volumes(Volumes { master: 0.0, music: 0.0, sfx: 0.0, user_music: 0.3, ..Volumes::default() });
    let clip = PcmClip::mono(24_000, &vec![0.1; 12_000]).unwrap();
    let opts = PcmOptions::new(Category::UserMusic);
    let own = engine.play_pcm(&clip, opts).unwrap();
    if !device_ready(&engine, own) {
        return;
    }
    let other = engine.play_pcm(&clip, opts).unwrap();
    assert!(until(|| engine.stats().started == 2));
    engine.set_voice_volume(other, 0.0);
    engine.set_volumes(Volumes { master: 0.0, user_music: 0.0, ..Volumes::default() });
    assert!(engine.is_playing(own));
    assert!(engine.is_playing(other));
    assert_finished_once(&engine, vec![own, other]);

    // Native 30/50 endpoints can be supplied explicitly; PcmSpatial still has its documented linear policy.
    let mut spatial = opts;
    spatial.spatial = Some(PcmSpatial { position: Vec3::X * 60.0, min_distance: 30.0, max_distance: 50.0 });
    let placed = engine.play_pcm(&clip, spatial).unwrap();
    assert!(until(|| engine.stats().started == 3));
    engine.set_position(placed, Vec3::X * 35.0);
    engine.stop(placed, true);
    engine.stop_all();
    assert_finished_once(&engine, vec![placed]);
    assert_eq!(engine.stats().started, 3);
    assert_eq!(engine.stats().music_ended, 0);
    assert!(engine.music_playing().is_none());
    assert_eq!(engine.stats().decoded_sounds, 0);
    assert_eq!(engine.stats().cache_bytes, 0);
    eprintln!("PCM native user-music route lifecycle: {:?}", engine.stats());
}

#[test]
#[ignore = "opens an audio device at master volume zero"]
fn pcm_stop_before_start_stop_all_and_reuse_release_all_eight_slots() {
    let engine = engine(64);
    let clip = PcmClip::mono(24_000, &vec![0.1; 240_000]).unwrap();
    let opts = PcmOptions::new(Category::Music);
    let first = engine.play_pcm(&clip, opts).unwrap();
    engine.stop(first, false); // stop queued before any worker/callback completion
    if !device_ready(&engine, first) {
        return;
    }
    assert_finished_once(&engine, vec![first]);

    let ids: Vec<_> = (0..8).map(|_| engine.play_pcm(&clip.clone(), opts).unwrap()).collect();
    assert!(until(|| engine.stats().started == 9));
    assert_eq!(engine.stats().active_voices, 8);
    assert_eq!(engine.stats().pcm_submissions, 8);
    assert_eq!(engine.clone().play_pcm(&clip, opts), Err(PcmError::Busy));
    let mut bad = opts;
    bad.volume = f32::INFINITY;
    assert_eq!(engine.play_pcm(&clip, bad), Err(PcmError::Options));
    engine.stop_all();
    engine.stop_all(); // repeated stop must not deliver duplicate completion or prolong the ramp
    assert_finished_once(&engine, ids);
    let again = engine.play_pcm(&clip, opts).unwrap();
    engine.stop(again, true);
    assert_finished_once(&engine, vec![again]);
    eprintln!("PCM bounded queue/stop lifecycle: {:?}", engine.stats());
}

#[test]
#[ignore = "needs the game pak and an audio device at master volume zero"]
fn pcm_and_original_events_share_global_voice_priority_and_limit() {
    let engine = engine(1);
    let click = "sounds/x2interface:interface:button_default";
    let priority = Library::global().event(click).unwrap().event().priority;
    let clip = PcmClip::mono(24_000, &vec![0.1; 240_000]).unwrap();
    let mut opts = PcmOptions::new(Category::Music);
    opts.priority = priority;
    let first = engine.play_pcm(&clip, opts).unwrap();
    if !device_ready(&engine, first) {
        return;
    }
    let event = engine.play_2d(click); // newer event steals equally important PCM
    assert!(until(|| !engine.is_playing(first)));
    assert!(until(|| engine.stats().started >= 2), "{:?}", engine.stats());
    let second = engine.play_pcm(&clip, opts).unwrap(); // newer PCM steals the event
    assert!(until(|| !engine.is_playing(event)));
    assert!(until(|| engine.stats().started >= 3));
    let third = engine.play_pcm(&clip, opts).unwrap(); // oldest PCM also loses ties
    assert!(until(|| !engine.is_playing(second)));
    opts.priority = priority.saturating_add(1);
    let refused = engine.play_pcm(&clip, opts).unwrap();
    assert!(until(|| !engine.is_playing(refused)));
    assert!(engine.is_playing(third));
    engine.stop(third, false);
    assert_finished_once(&engine, vec![first, event, second, third, refused]);
    assert_eq!((engine.stats().stolen, engine.stats().refused), (3, 1));
    eprintln!("PCM/event shared admission: {:?}", engine.stats());
}
