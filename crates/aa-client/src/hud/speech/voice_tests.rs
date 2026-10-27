//! The used single-slot controller, without an audio device or decode worker.
use super::*;
use std::cell::RefCell;
use aa_audio::Voice;

#[test]
fn admitted_replacement_stops_before_sampling_and_keeps_the_point_snapshot() {
    let calls = RefCell::new(Vec::new());
    let mut slot = EffectVoice(Some(Voice(1)));
    slot.replace(Some("voice/event"), |id| calls.borrow_mut().push(("stop", id.0)),
        || { calls.borrow_mut().push(("point", 0)); Some(Vec3::new(2.0, 3.0, 4.0)) },
        |path, point| {
            assert_eq!(path, "voice/event");
            assert_eq!(point, Vec3::new(2.0, 3.0, 4.0));
            calls.borrow_mut().push(("play", 2)); Voice(2)
        });
    assert_eq!(*calls.borrow(), [("stop", 1), ("point", 0), ("play", 2)]);
    assert_eq!(slot.0, Some(Voice(2)));
    slot.reap(|id| { assert_eq!(id, Voice(2)); true });
    assert_eq!(slot.0, Some(Voice(2)), "completion of another voice is irrelevant");
    slot.reap(|id| { assert_eq!(id, Voice(2)); false });
    assert_eq!(slot.0, None);
}

#[test]
fn cache_miss_preserves_old_but_an_admitted_missing_point_cancels_it() {
    let mut slot = EffectVoice(Some(Voice(1)));
    slot.replace(None, |_| panic!("miss must not stop"), || panic!("miss must not sample"), |_, _| panic!("miss must not play"));
    assert_eq!(slot.0, Some(Voice(1)));
    let stopped = RefCell::new(None);
    slot.replace(Some("admitted"), |id| *stopped.borrow_mut() = Some(id), || None, |_, _| panic!("no point"));
    assert_eq!(*stopped.borrow(), Some(Voice(1)));
    assert_eq!(slot.0, None);
}

#[test]
fn original_voice_cache_misses_and_hits_use_the_existing_pack_lookup() -> Result<(), Box<dyn std::error::Error>> {
    let db = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.into());
    if !std::path::Path::new(&db).exists() {
        return Ok(());
    }
    let packs = aa_audio::SoundPacks::open(&db)?;
    assert!(packs.path("voice", "bubble_effect_1").is_none());
    assert!(packs.path("voice", "bubble_effect_3").is_none());
    for id in [969, 925, 923, 2939] {
        let path = packs.path("voice", &format!("bubble_effect_{id}")).unwrap();
        assert!(path.contains("anthalon"), "{id}: {path}");
    }
    Ok(())
}
