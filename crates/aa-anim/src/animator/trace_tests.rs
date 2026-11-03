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
