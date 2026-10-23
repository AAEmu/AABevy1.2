//! The engine with a real device and the real pak, silent (master volume 0): voices are tracked while they live, long loops
//! stream and the music reports its ending. `cargo test -p aa-audio --test engine -- --ignored`. A machine without an audio
//! device skips them.

use std::time::{Duration, Instant};

use aa_audio::{Engine, PlayOptions, Settings, Volumes};
use bevy::math::Vec3;

fn engine() -> Engine {
    let engine = Engine::start(Settings { audible: true, ..Settings::default() });
    engine.set_volumes(Volumes { master: 0.0, ..Volumes::default() });
    engine
}

fn until(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
#[ignore = "needs the game pak; opens no audio device"]
fn failed_original_requests_finish_once_without_a_device() {
    let engine = Engine::start(Settings { audible: true, max_voices: 0, ..Settings::default() });
    let missing = engine.play_2d("sounds/x2voice_prologue:system:6170");
    let refused = engine.play_2d("sounds/x2interface:interface:button_default");
    let culled = engine.play_at("sounds/x2eventsounds:quest:cat_cry_2", Vec3::splat(1000.0));
    assert!(engine.wait_idle(Duration::from_secs(5)), "{:?}", engine.stats());
    assert!(!engine.is_playing(missing) && !engine.is_playing(refused) && !engine.is_playing(culled));
    assert_eq!(engine.take_finished(), [missing, refused, culled]);
    assert!(engine.take_finished().is_empty(), "completion is delivered exactly once");
    let stats = engine.stats();
    assert_eq!((stats.unknown_events, stats.refused, stats.culled), (1, 1, 1));
    assert_eq!(stats.started, 0);
    assert_eq!(stats.decoded_sounds, 0);
    assert!(stats.device_error.is_none());
}

#[test]
#[ignore = "needs the game pak and an audio device"]
fn a_voice_is_playing_until_it_ends() {
    let engine = engine();
    let voice = engine.play_2d("sounds/x2interface:interface:button_default");
    assert!(engine.is_playing(voice), "asked for, so it counts at once");
    assert!(until(Duration::from_secs(5), || engine.stats().device_error.is_some() || engine.stats().started > 0), "{:?}", engine.stats());
    if let Some(error) = engine.stats().device_error {
        eprintln!("skipped device playback: {error}");
        return;
    }
    assert!(until(Duration::from_secs(10), || !engine.is_playing(voice)), "a click ends by itself");
    assert!(engine.take_finished().contains(&voice));

    // an event that does not exist is not left behind
    let none = engine.play_2d("sounds/x2voice_prologue:system:6170");
    assert!(until(Duration::from_secs(5), || !engine.is_playing(none)));

    // stopping a loop ends it
    let wind = engine.play_2d("sounds/x2ambience:world2_day:field_day2");
    assert!(until(Duration::from_secs(5), || engine.stats().started > 1));
    assert!(engine.is_playing(wind));
    engine.stop(wind, false);
    assert!(until(Duration::from_secs(5), || !engine.is_playing(wind)));
    eprintln!("voice lifecycle verified: {:?}", engine.stats());
}

#[test]
#[ignore = "needs the game pak and an audio device"]
fn long_loops_stream_instead_of_filling_the_cache() {
    let engine = engine();
    // the combat music is a looping playlist of tracks of minutes
    let music = engine.play_2d("sounds/x2music:conditions_bgm:battle_normal");
    assert!(until(Duration::from_secs(5), || engine.stats().device_error.is_some() || engine.stats().streaming > 0), "{:?}", engine.stats());
    if let Some(error) = engine.stats().device_error {
        eprintln!("skipped device playback: {error}");
        return;
    }
    let s = engine.stats();
    assert!(s.streaming > 0, "{s:?}");
    // none of the long ones was decoded whole (minutes of stereo would be tens of MiB)
    assert!(s.cache_bytes < 8 << 20, "the cache holds {} bytes", s.cache_bytes);
    eprintln!("long loop verified: {s:?}");
    engine.stop(music, false);
}

#[test]
#[ignore = "needs the game pak and an audio device"]
fn music_reports_what_plays_and_when_a_track_ends() {
    let engine = engine();
    engine.play_music("sounds/x2music:screen:login_2013_07", Duration::ZERO, Duration::ZERO);
    assert!(until(Duration::from_secs(5), || engine.stats().device_error.is_some() || engine.music_playing().is_some()), "{:?}", engine.stats());
    if let Some(error) = engine.stats().device_error {
        eprintln!("skipped device playback: {error}");
        return;
    }
    assert_eq!(engine.music_playing().as_deref(), Some("sounds/x2music:screen:login_2013_07"));
    assert!(!engine.wait_idle(Duration::from_millis(50)), "a playing music stream is not idle");
    engine.stop_music(Duration::from_millis(10));
    assert!(until(Duration::from_secs(2), || engine.music_playing().is_none()));
    // a stop is not an end
    assert_eq!(engine.stats().music_ended, 0);
    eprintln!("music stop verified: {:?}", engine.stats());

    // A short, original non-looping event exercises the same streaming path without waiting
    // minutes for a zone track. No fabricated sample or modified project is needed.
    let click = "sounds/x2interface:interface:button_default";
    engine.play_music_event(click);
    assert!(until(Duration::from_secs(5), || engine.music_playing().as_deref() == Some(click)), "{:?}", engine.stats());
    assert!(until(Duration::from_secs(5), || engine.stats().music_ended == 1), "{:?}", engine.stats());
    assert!(engine.music_playing().is_none());
    assert_eq!(engine.stats().streaming, 0);
    eprintln!("natural music completion verified: {:?}", engine.stats());
}

#[test]
#[ignore = "needs the game pak and an audio device; master zero"]
fn original_highpass_assets_stream_move_and_finish_normally() {
    let engine = engine();
    for path in ["sounds/x2ambience:login_stage:countryside02", "sounds/x2ambience:login_stage:mar03",
        "sounds/x2ambience:world2_day:countryside02", "sounds/x2ambience:world2_night:wind_night1",
        "sounds/x2weapons:x2weapons:shield_metal"]
    {
        let before = engine.stats().started;
        let spatial = path.contains("shield_metal");
        let opts = PlayOptions { params: vec![("daylight".into(), if path.contains("world2_night") { 0.0 } else { 1.0 })], ..PlayOptions::default() };
        let id = engine.play_with(path, spatial.then_some(Vec3::X * 3.0), opts);
        assert!(until(Duration::from_secs(5), || engine.stats().device_error.is_some() || engine.stats().started > before), "{path}: {:?}", engine.stats());
        if let Some(error) = engine.stats().device_error { eprintln!("skipped device playback: {error}"); return; }
        if !spatial { assert!(engine.stats().streaming > 0, "original long high-pass ambience streams: {path}"); }
        engine.set_param(id, "battle", 1.0);
        engine.set_param(id, "pierce", 1.0);
        engine.set_listener(Vec3::X, bevy::math::Quat::IDENTITY);
        if spatial {
            engine.set_position(id, Vec3::X * -3.0);
            assert!(until(Duration::from_secs(5), || !engine.is_playing(id)), "one-shot sample clock completes");
        } else {
            engine.stop(id, false);
            assert!(until(Duration::from_secs(5), || !engine.is_playing(id)), "loop stop completes");
        }
        assert!(engine.wait_idle(Duration::from_secs(5)));
        assert_eq!(engine.take_finished().iter().filter(|&&v| v == id).count(), 1);
        assert!(engine.take_finished().is_empty());
    }
    let stats = engine.stats();
    assert_eq!((stats.started, stats.decode_failures, stats.active_voices, stats.streaming, stats.pcm_submissions), (5, 0, 0, 0, 0));
    eprintln!("original five-asset highpass device lifecycle: {stats:?}");
}
