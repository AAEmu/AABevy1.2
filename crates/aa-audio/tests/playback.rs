//! Actual, unmodified FEV loops on the device mixer. Master is positive but below
//! the engine's silence floor, so native-style gain comparisons remain meaningful.

use std::time::{Duration, Instant};

use aa_audio::{Category, Engine, Library, PlayOptions, Settings, Voice, Volumes};
use bevy::math::{Quat, Vec3};

const STEAL: &str = "sounds/x2eventsounds:quest_elf:stone_mark_beam";
const CONDITIONAL: &str = "sounds/x2eventsounds:prefab:mining_machinery_idle";

fn until(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if check() { return true; }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn engine() -> (Engine, Volumes) {
    let engine = Engine::start(Settings::default());
    let volumes = Volumes { master: 0.0001, ..Volumes::default() };
    engine.set_volumes(volumes);
    (engine, volumes)
}

fn play(engine: &Engine, path: &str, volume: f32, category: Category, distance: f32) -> Voice {
    engine.play_with(path, Some(Vec3::X * distance), PlayOptions { volume, category: Some(category), ..PlayOptions::default() })
}

fn started(engine: &Engine, count: u64) -> bool {
    assert!(until(Duration::from_secs(5), || engine.stats().started >= count || engine.stats().device_error.is_some()), "{:?}", engine.stats());
    if let Some(error) = engine.stats().device_error {
        eprintln!("SKIPPED device-backed FEV policy playback: {error}");
        return false;
    }
    true
}

fn ended(engine: &Engine, id: Voice) {
    // Both originals have authored fades >= 600 ms. A steal bypasses them.
    assert!(until(Duration::from_millis(500), || !engine.is_playing(id)), "steal/refusal did not finish {id:?}: {:?}", engine.stats());
}

fn finish(engine: &Engine, mut ids: Vec<Voice>) {
    engine.stop_all();
    assert!(engine.wait_idle(Duration::from_secs(5)), "{:?}", engine.stats());
    assert!(ids.iter().all(|&id| !engine.is_playing(id)));
    let mut finished = engine.take_finished();
    ids.sort_by_key(|v| v.0);
    finished.sort_by_key(|v| v.0);
    assert_eq!(finished, ids, "every stolen/refused/stopped request completes exactly once");
    assert!(engine.take_finished().is_empty());
    assert_eq!(engine.stats().decode_failures, 0);
    assert!(engine.stats().device_error.is_none());
}

#[test]
#[ignore = "needs the game pak and an audio device; output is below the engine silence floor"]
fn original_policy3_tracks_live_volume_pool_ties_and_muting() {
    let event = Library::global().event(STEAL).unwrap();
    assert_eq!((event.event().playback_behaviour, event.event().max_playbacks), (3, 3));
    assert_eq!((event.event().fade_in_ms, event.event().fade_out_ms), (500, 800));
    let (engine, mut volumes) = engine();
    let a = play(&engine, STEAL, 0.8, Category::Sfx, 0.0);
    if !started(&engine, 1) { return; }
    let b = play(&engine, STEAL, 0.2, Category::Sfx, 0.0);
    let c = play(&engine, STEAL, 0.5, Category::Sfx, 0.0);
    assert!(started(&engine, 3));
    assert!([a, b, c].iter().all(|&id| engine.is_playing(id)), "three actual overlapping loops");
    // The newest voice becomes quietest. A still-quieter newcomer is admitted.
    engine.set_voice_volume(c, 0.005);
    let d = play(&engine, STEAL, 0.001, Category::Sfx, 0.0);
    ended(&engine, c);
    assert!(started(&engine, 4));
    assert!(engine.is_playing(a) && engine.is_playing(b) && engine.is_playing(d));
    // Initial native slot order is 1,2,0: d inherited c's slot 0. Equal
    // minima in slots 0 and 2 keep d, although b (slot 2) is older.
    engine.set_voice_volume(b, 0.001);
    let e = play(&engine, STEAL, 0.5, Category::Sfx, 0.0);
    ended(&engine, d);
    assert!(started(&engine, 5));
    assert!(engine.is_playing(b));
    // An all-equal zero metric takes the oldest fallback (a, in slot 1).
    volumes.sfx = 0.0;
    engine.set_volumes(volumes);
    let f = play(&engine, STEAL, 1.0, Category::Sfx, 0.0);
    ended(&engine, a);
    assert!(started(&engine, 6));
    assert!(engine.is_playing(b) && engine.is_playing(e) && engine.is_playing(f));
    assert_eq!((engine.stats().stolen, engine.stats().refused), (3, 0));
    finish(&engine, vec![a, b, c, d, e, f]);
    eprintln!("original policy 3 volume/pool/category verified: {:?}", engine.stats());
}

#[test]
#[ignore = "needs the game pak and an audio device; output is below the engine silence floor"]
fn original_policy5_compares_incoming_and_runtime_category_gains() {
    let event = Library::global().event(CONDITIONAL).unwrap();
    assert_eq!((event.event().playback_behaviour, event.event().max_playbacks), (5, 2));
    assert_eq!((event.event().fade_in_ms, event.event().fade_out_ms), (200, 600));
    let (engine, mut volumes) = engine();
    let a = play(&engine, CONDITIONAL, 0.8, Category::Sfx, 0.0);
    if !started(&engine, 1) { return; }
    let b = play(&engine, CONDITIONAL, 0.2, Category::Ui, 0.0);
    assert!(started(&engine, 2));
    let equal = play(&engine, CONDITIONAL, 0.2, Category::Ui, 0.0);
    let quieter = play(&engine, CONDITIONAL, 0.1, Category::Ui, 0.0);
    ended(&engine, equal);
    ended(&engine, quieter);
    assert!(engine.is_playing(a) && engine.is_playing(b));
    let c = play(&engine, CONDITIONAL, 0.3, Category::Ui, 0.0);
    ended(&engine, b);
    assert!(started(&engine, 3));
    engine.set_voice_volume(a, 0.05);
    let d = play(&engine, CONDITIONAL, 0.1, Category::Sfx, 0.0);
    ended(&engine, a);
    assert!(started(&engine, 4));
    volumes.sfx = 0.0;
    engine.set_volumes(volumes);
    let muted = play(&engine, CONDITIONAL, 1.0, Category::Sfx, 0.0);
    ended(&engine, muted);
    assert!(engine.is_playing(c) && engine.is_playing(d));
    let e = play(&engine, CONDITIONAL, 0.05, Category::Ui, 0.0);
    ended(&engine, d); // lower user volume, but its live category is louder
    assert!(started(&engine, 5));
    volumes.master = 0.0;
    engine.set_volumes(volumes);
    let all_muted = play(&engine, CONDITIONAL, 1.0, Category::Ui, 0.0);
    ended(&engine, all_muted);
    assert!(until(Duration::from_secs(1), || engine.stats().refused == 4));
    assert!(engine.is_playing(c) && engine.is_playing(e));
    assert_eq!((engine.stats().stolen, engine.stats().refused), (3, 4));
    finish(&engine, vec![a, b, equal, quieter, c, d, muted, e, all_muted]);
    eprintln!("original policy 5 admission/volume/category/master verified: {:?}", engine.stats());
}

#[test]
#[ignore = "needs the game pak and an audio device; output is below the engine silence floor"]
fn original_policy5_refreshes_source_and_listener_distance_before_admission() {
    let (engine, _) = engine();
    let a = play(&engine, CONDITIONAL, 1.0, Category::Sfx, 1.0);
    if !started(&engine, 1) { return; }
    let b = play(&engine, CONDITIONAL, 1.0, Category::Sfx, 10.0);
    assert!(started(&engine, 2));
    engine.set_position(a, Vec3::X * 14.0);
    let c = play(&engine, CONDITIONAL, 1.0, Category::Sfx, 2.0);
    ended(&engine, a);
    assert!(started(&engine, 3));
    assert!(engine.is_playing(b));
    engine.set_listener(Vec3::X * 10.0, Quat::IDENTITY);
    let d = play(&engine, CONDITIONAL, 1.0, Category::Sfx, 10.0);
    ended(&engine, c);
    assert!(started(&engine, 4));
    assert!(engine.is_playing(b) && engine.is_playing(d));
    assert_eq!((engine.stats().stolen, engine.stats().refused), (2, 0));
    finish(&engine, vec![a, b, c, d]);
    eprintln!("original policy 5 source/listener distance verified: {:?}", engine.stats());
}
