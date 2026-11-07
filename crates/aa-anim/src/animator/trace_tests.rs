use super::*;
use crate::Cal;

const SKELETON: &str = "objects/characters/nuian/male/nude/nu_m_base.chr";
const CAL: &str = "objects/characters/nuian/male/nude/nu_m_base.cal";

fn bound(rig: &Rig, library: &ClipLibrary, cal: &Cal, name: &str) -> BoundClip {
    let entry = cal.animation(name).unwrap();
    let options = SampleOptions { position_scale: entry.position_scale(cal.base_height), ..Default::default() };
    let mut clip = library.bound(rig, &entry.path, options).unwrap();
    clip.name = Some(name.into());
    clip
}

#[test]
fn every_start_is_recorded_once_the_trace_is_on() {
    if !aa_cry::pak_available() {
        return;
    }
    trace::enable();
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let (library, cal) = (ClipLibrary::default(), Cal::shared(CAL).unwrap());
    let idle = bound(&rig, &library, &cal, "fist_ba_relaxed_idle");
    let run = bound(&rig, &library, &cal, "fist_mo_normal_run_f");
    let cast = bound(&rig, &library, &cal, "bow_co_sk_cast_ub");

    let mut a = Animator::new(rig.clone(), Vec::new());
    assert!(!a.has_starts());
    a.play(idle.clone(), true, 0.25);
    a.play_once_then(run.clone(), idle.clone(), 0.8);
    a.play_layer(BoneMask::upper_body(&rig).unwrap(), cast.clone(), false, 0.1);
    // the one-shot runs out and the animator goes back to the idle by itself
    a.update(run.clip.duration + 0.1);
    assert!(a.has_starts());
    let starts = a.take_starts();
    assert!(!a.has_starts() && a.take_starts().is_empty(), "taking them empties the record");

    let summary: Vec<_> = starts.iter().map(|s| (s.layer, s.looping, s.blend, s.name.as_deref())).collect();
    assert_eq!(
        summary,
        [
            (trace::BASE, true, 0.25, Some("fist_ba_relaxed_idle")),
            (trace::BASE, false, 0.8, Some("fist_mo_normal_run_f")),
            (trace::UPPER, false, 0.1, Some("bow_co_sk_cast_ub")),
            (trace::BASE, true, 0.2, Some("fist_ba_relaxed_idle")),
        ],
        "the base clips, the layer, and the return clip the animator started by itself"
    );
    assert_eq!(starts[2].file, cast.source, "the pak path of the clip");
    assert!(starts[2].file.is_some());
    assert!(starts.windows(2).all(|w| w[0].wall_us <= w[1].wall_us) && starts[0].wall_us > 1_600_000_000_000_000);

    // a start that gameplay framehold refuses is not a start
    a.set_framehold(true);
    a.play(run, true, 0.0);
    assert!(!a.has_starts());
}

/// A seconds-mode random idle: a variant 3.0 s into the idle (or its first one later), faded in and back out over its own blends,
/// the idle's clock restarting when it comes back, and never the same variant twice in a row.
#[test]
fn a_timed_random_idle_plays_another_variant_every_interval() {
    if !aa_cry::pak_available() {
        return;
    }
    trace::enable();
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let (library, cal) = (ClipLibrary::default(), Cal::shared(CAL).unwrap());
    let idle = bound(&rig, &library, &cal, "fist_ba_relaxed_idle");
    let variants = vec![bound(&rig, &library, &cal, "fist_ba_relaxed_idle_rand_1"), bound(&rig, &library, &cal, "fist_ba_relaxed_idle_rand_2")];
    let lengths: Vec<f32> = variants.iter().map(|v| v.clip.duration).collect();

    let mut a = Animator::new(rig.clone(), Vec::new());
    a.play(idle.clone(), true, 0.2);
    a.random_idle = Some(RandomIdle::every_secs(variants, 3.0, 0.2, 0.15, 11).first_after(3.12));
    a.take_starts();
    let dt = 1.0 / 100.0;
    let mut t = 0.0f32;
    let mut seen: Vec<(f32, Option<String>, bool, f32)> = Vec::new();
    while t < 3.12 + lengths[0].max(lengths[1]) + 3.0 + 0.5 {
        a.update(dt);
        t += dt;
        for s in a.take_starts() {
            seen.push((t, s.name.as_deref().map(str::to_string), s.looping, s.blend));
        }
    }
    let first = &seen[0];
    assert!((first.0 - 3.12).abs() < 0.02, "the first variant at 3.12 s: {seen:?}");
    assert!(first.1.as_deref().is_some_and(|n| n.starts_with("fist_ba_relaxed_idle_rand_")) && !first.2 && first.3 == 0.2);
    let back = &seen[1];
    assert_eq!((back.1.as_deref(), back.2, back.3), (Some("fist_ba_relaxed_idle"), true, 0.15), "back to the idle over the variant's blend out");
    let second = &seen[2];
    assert!((second.0 - back.0 - 3.0).abs() < 0.02, "the next one 3.0 s after the idle came back: {seen:?}");
    assert_ne!(second.1, first.1, "another variant");
}
