use super::*;
use crate::Cal;
use std::time::Duration;

const SKELETON: &str = "objects/characters/nuian/male/nude/nu_m_base.chr";
const CAL: &str = "objects/characters/nuian/male/nude/nu_m_base.cal";

fn clips(rig: &Rig) -> [BoundClip; 3] {
    let cal = Cal::shared(CAL).unwrap();
    let library = ClipLibrary::default();
    [
        "fist_ba_relaxed_idle",
        "fist_mo_normal_run_f",
        "bow_co_sk_cast_ub",
    ]
    .map(|name| {
        let entry = cal.animation(name).unwrap();
        library
            .bound(
                rig,
                &entry.path,
                SampleOptions {
                    position_scale: entry.position_scale(cal.base_height),
                    ..Default::default()
                },
            )
            .unwrap()
    })
}

fn configured(rig: &Arc<Rig>, bones: Vec<Entity>) -> Animator {
    let [idle, run, cast] = clips(rig);
    let mut a = Animator::new(rig.clone(), bones);
    a.play(idle.clone(), true, 0.0);
    a.update(0.37);
    a.play_once_then(run, idle, 0.8);
    a.play_layer(BoneMask::upper_body(rig).unwrap(), cast.clone(), true, 0.5);
    a.random_idle = Some(RandomIdle::new(vec![cast], 2, 7));
    a.update(0.13);
    a
}

#[test]
fn framehold_atomically_rejects_all_starts_without_replaying_them_on_release() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let [idle, run, cast] = clips(&rig);
    let mut a = configured(&rig, Vec::new());
    let mut control = configured(&rig, Vec::new());
    assert!(a.starts_enabled());
    a.set_framehold(true);
    assert!(!a.paused);
    assert!(!a.starts_enabled());
    let held = a.playback_snapshot();
    a.play(cast.clone(), true, 0.0);
    assert_eq!(a.playback_snapshot(), held);
    a.play_layer(BoneMask::upper_body(&rig).unwrap(), run.clone(), false, 0.0);
    assert_eq!(a.playback_snapshot(), held);
    a.play_once_then(cast, run, 0.0);
    assert_eq!(a.playback_snapshot(), held);
    for _ in 0..600 {
        a.update(1.0 / 60.0);
    }
    assert_eq!(a.playback_snapshot(), held);
    a.set_framehold(false);
    assert_eq!(a.playback_snapshot(), control.playback_snapshot());
    a.update(0.1);
    control.update(0.1);
    assert_eq!(a.playback_snapshot(), control.playback_snapshot());
    // Rejected requests are dropped, not queued for release. A new explicit start succeeds.
    a.play(idle.clone(), true, 0.2);
    assert!(Arc::ptr_eq(
        &a.current.as_ref().unwrap().clip.clip,
        &idle.clip
    ));
    assert!(a.then.is_none());
}

#[test]
fn framehold_release_preserves_manual_pause_and_zero_speed_preview_admission() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let [idle, run, cast] = clips(&rig);
    let mask = BoneMask::upper_body(&rig).unwrap();
    let mut a = Animator::new(rig.clone(), Vec::new());
    a.paused = true;
    a.speed = 0.0;
    a.play(run.clone(), true, 0.0);
    a.play_layer(mask.clone(), cast.clone(), true, 0.2);
    a.play_once_then(run.clone(), idle.clone(), 0.2);
    assert!(a.current.is_some() && a.then.is_some() && a.has_layer());
    a.set_framehold(true);
    a.set_framehold(false);
    assert!(a.paused && a.starts_enabled());
    assert_eq!(a.speed, 0.0);
    a.play(run.clone(), true, 0.0);
    a.stop_layer(0.0);
    a.current.as_mut().unwrap().time = 0.31;
    a.update(2.0);
    let mut expected = rig.bind.clone();
    expected.sample(&rig, &run.clip, &run.binding, 0.31, run.options);
    assert_eq!(a.pose().rot, expected.rot);
    assert_eq!(a.pose().pos, expected.pos);
    assert_eq!(a.current.as_ref().unwrap().time, 0.31);
    // Zero speed parks only the base clock; starts and transition/layer clocks work.
    a.paused = false;
    a.play(idle, true, 0.4);
    a.play_layer(mask, cast, true, 0.4);
    a.update(0.1);
    assert_eq!(a.current.as_ref().unwrap().time, 0.0);
    assert_eq!(a.fade.as_ref().unwrap().elapsed, 0.1);
    assert_eq!(a.layer.as_ref().unwrap().playing.time, 0.1);
}

#[test]
fn stop_remains_available_while_frameheld_and_timed_stop_resumes_on_release() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let mut a = configured(&rig, Vec::new());
    a.set_framehold(true);
    a.stop_layer(0.3);
    assert!(a.layer.as_ref().unwrap().stopping.is_some());
    a.update(0.0);
    let stopped = a.playback_snapshot();
    a.update(2.0);
    assert_eq!(a.playback_snapshot(), stopped);
    a.set_framehold(false);
    a.update(0.4);
    assert!(!a.has_layer());
    let [_, _, cast] = clips(&rig);
    a.play_layer(BoneMask::upper_body(&rig).unwrap(), cast, true, 0.1);
    a.set_framehold(true);
    a.stop_layer(0.0);
    assert!(!a.has_layer());
    assert!(!a.starts_enabled());
}

#[test]
fn anim_plugin_keeps_original_bones_held_and_rejects_starts_until_release() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let [idle, run, cast] = clips(&rig);
    let mut app = App::new();
    app.insert_resource(Time::<()>::default())
        .add_plugins((bevy::transform::TransformPlugin, AnimPlugin));
    let root = app
        .world_mut()
        .spawn((Transform::default(), GlobalTransform::default()))
        .id();
    let bones = spawn_bones(&mut app.world_mut().commands(), root, &rig);
    app.world_mut().flush();
    let mut a = configured(&rig, bones.clone());
    a.set_framehold(true);
    app.world_mut().entity_mut(root).insert(a);
    let step = |app: &mut App, dt| {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(dt));
        app.update();
    };
    step(&mut app, 0.0);
    let frozen = bones
        .iter()
        .map(|e| *app.world().get::<Transform>(*e).unwrap())
        .collect::<Vec<_>>();
    let state = app
        .world()
        .get::<Animator>(root)
        .unwrap()
        .playback_snapshot();
    {
        let mut a = app.world_mut().get_mut::<Animator>(root).unwrap();
        a.play(cast.clone(), true, 0.0);
        a.play_layer(BoneMask::upper_body(&rig).unwrap(), run, false, 0.0);
        a.play_once_then(cast, idle, 0.0);
    }
    for dt in [0.016, 0.2, 0.0, 0.5] {
        step(&mut app, dt);
        assert_eq!(
            app.world()
                .get::<Animator>(root)
                .unwrap()
                .playback_snapshot(),
            state
        );
        for (i, e) in bones.iter().enumerate() {
            assert_eq!(*app.world().get::<Transform>(*e).unwrap(), frozen[i]);
        }
    }
    app.world_mut()
        .get_mut::<Animator>(root)
        .unwrap()
        .set_framehold(false);
    step(&mut app, 0.1);
    assert!(app.world().get::<Animator>(root).unwrap().starts_enabled());
    assert!(
        bones
            .iter()
            .enumerate()
            .any(|(i, e)| *app.world().get::<Transform>(*e).unwrap() != frozen[i])
    );
}
