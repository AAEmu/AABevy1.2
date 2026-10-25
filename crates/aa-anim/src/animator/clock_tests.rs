use super::*;
use crate::Cal;
use std::time::Duration;

#[path = "native_clock_cases.rs"]
mod native;

const SKELETON: &str = "objects/characters/nuian/male/nude/nu_m_base.chr";

fn clips(rig: &Rig) -> [BoundClip; 3] {
    let cal = Cal::shared("objects/characters/nuian/male/nude/nu_m_base.cal").unwrap();
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

fn fading(rig: &Arc<Rig>, bones: Vec<Entity>) -> Animator {
    let [idle, run, cast] = clips(rig);
    let mut animator = Animator::new(rig.clone(), bones);
    animator.play(idle, true, 0.0);
    animator.update(0.37);
    animator.play(run, true, 0.5);
    animator.play_layer(BoneMask::upper_body(rig).unwrap(), cast, true, 0.4);
    animator.update(0.125);
    animator
}

fn assert_pose_eq(actual: &Pose, expected: &Pose) {
    assert_eq!(actual.rot, expected.rot);
    assert_eq!(actual.pos, expected.pos);
}

#[test]
fn zero_delta_does_not_move_an_original_clip_transition() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let mut animator = fading(&rig, Vec::new());
    let state = animator.playback_snapshot();
    for _ in 0..1000 {
        animator.update(0.0);
    }
    assert_eq!(animator.fade.as_ref().unwrap().elapsed, 0.125);
    assert!(animator.playback_snapshot() == state);
}

#[test]
fn framehold_preserves_base_transition_and_stopping_layer_then_resumes() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let mut held = fading(&rig, Vec::new());
    let mut control = fading(&rig, Vec::new());
    held.stop_layer(0.3);
    control.stop_layer(0.3);
    held.update(0.05);
    control.update(0.05);
    held.paused = true; // aa-client combat::buffs::refreeze's animation boundary.
    let state = held.playback_snapshot();
    for _ in 0..600 {
        held.update(1.0 / 60.0);
    }
    assert_eq!(held.fade.as_ref().unwrap().elapsed, 0.175);
    assert!(held.playback_snapshot() == state);
    held.paused = false;
    held.update(0.1);
    control.update(0.1);
    assert!(held.playback_snapshot() == control.playback_snapshot());
    held.update(0.5);
    control.update(0.5);
    assert!(!held.has_layer());
    assert!(held.fade.is_none());
    assert_pose_eq(held.pose(), control.pose());
}

#[test]
fn zero_clip_speed_still_crossfades_and_plays_the_upper_body_layer() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let mut animator = fading(&rig, Vec::new());
    let time = animator.current.as_ref().unwrap().time;
    let layer_time = animator.layer.as_ref().unwrap().playing.time;
    let before = animator.pose().clone();
    animator.speed = 0.0;
    animator.update(0.125);
    assert_eq!(animator.current.as_ref().unwrap().time, time);
    assert_eq!(animator.fade.as_ref().unwrap().elapsed, 0.25);
    assert_eq!(
        animator.layer.as_ref().unwrap().playing.time,
        layer_time + 0.125
    );
    assert!(
        animator
            .pose()
            .rot
            .iter()
            .zip(&before.rot)
            .any(|(a, b)| a.angle_between(*b) > 0.01)
    );
    animator.stop_layer(0.0);
    animator.update(0.25);
    assert!(animator.fade.is_none());
    let mut sampled = rig.bind.clone();
    animator
        .current
        .as_ref()
        .unwrap()
        .sample(&mut sampled, &rig);
    assert_pose_eq(animator.pose(), &sampled);
}

#[test]
fn transition_clock_uses_real_seconds_at_slow_fast_and_submillisecond_steps() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    for speed in [0.0, 0.5, 1.0, 2.0] {
        let mut animator = fading(&rig, Vec::new());
        let time = animator.current.as_ref().unwrap().time;
        animator.speed = speed;
        animator.update(0.00001);
        assert_eq!(animator.fade.as_ref().unwrap().elapsed, 0.125 + 0.00001);
        assert_eq!(
            animator.current.as_ref().unwrap().time,
            time + speed * 0.00001
        );
        animator.update(0.125);
        assert_eq!(
            animator.fade.as_ref().unwrap().elapsed,
            (0.125 + 0.00001) + 0.125
        );
    }
}

#[test]
fn original_fifo_clock_goldens_drive_real_animator_clips() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let [idle, run, _] = clips(&rig);
    assert!(idle.clip.duration > 0.7);
    for &(paused, speed, dt, priority, normalized_time) in native::CLOCKS {
        let mut animator = Animator::new(rig.clone(), Vec::new());
        animator.play(run.clone(), true, 0.0);
        animator.update(0.37);
        animator.play(idle.clone(), true, 0.5);
        animator.fade.as_mut().unwrap().elapsed = 0.125;
        animator.current.as_mut().unwrap().time = 0.4;
        animator.paused = paused;
        animator.speed = speed;
        animator.update(dt);
        // Native fixtures use a two-second clip and a half-second transition.
        // Compare clocks only: native FIFO also shapes weights and samples both clips;
        // Animator's existing frozen-source, linear pose blend is a separate gap.
        let elapsed = animator.fade.as_ref().unwrap().elapsed;
        assert!(
            (elapsed - priority * 0.5).abs() < 1e-7,
            "paused={paused} speed={speed} dt={dt}: {elapsed}"
        );
        let time = animator.current.as_ref().unwrap().time;
        assert!(
            (time - normalized_time * 2.0).abs() < 1e-7,
            "paused={paused} speed={speed} dt={dt}: {time}"
        );
    }
}

#[test]
fn paused_manual_time_can_still_be_sampled() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let [_, run, _] = clips(&rig);
    let mut animator = Animator::new(rig.clone(), Vec::new());
    animator.play(run, true, 0.0);
    animator.paused = true;
    animator.current.as_mut().unwrap().time = 0.31;
    animator.update(0.0);
    let first = animator.pose().clone();
    animator.current.as_mut().unwrap().time = 0.59;
    animator.update(0.0);
    let mut expected = rig.bind.clone();
    animator
        .current
        .as_ref()
        .unwrap()
        .sample(&mut expected, &rig);
    assert_pose_eq(animator.pose(), &expected);
    assert_ne!(animator.pose().rot, first.rot);
}

#[test]
fn anim_plugin_holds_and_resumes_original_bone_transforms_during_a_transition() {
    if !aa_cry::pak_available() {
        return;
    }
    let rig = crate::pose::shared_rig(SKELETON).unwrap();
    let mut app = App::new();
    app.insert_resource(Time::<()>::default())
        .add_plugins((bevy::transform::TransformPlugin, AnimPlugin));
    let root = app
        .world_mut()
        .spawn((Transform::default(), GlobalTransform::default()))
        .id();
    let bones = spawn_bones(&mut app.world_mut().commands(), root, &rig);
    app.world_mut().flush();
    let mut animator = fading(&rig, bones.clone());
    animator.paused = true;
    app.world_mut().entity_mut(root).insert(animator);
    let step = |app: &mut App, dt| {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(dt));
        app.update();
    };
    step(&mut app, 0.0);
    let locals = || {
        bones
            .iter()
            .map(|e| *app.world().get::<Transform>(*e).unwrap())
            .collect::<Vec<_>>()
    };
    let frozen = locals();
    let state = app
        .world()
        .get::<Animator>(root)
        .unwrap()
        .playback_snapshot();
    for dt in [0.0, 0.016, 0.2, 0.0, 0.5] {
        step(&mut app, dt);
        let a = app.world().get::<Animator>(root).unwrap();
        assert!(a.playback_snapshot() == state);
        for (i, e) in bones.iter().enumerate() {
            assert_eq!(*app.world().get::<Transform>(*e).unwrap(), frozen[i]);
        }
    }
    app.world_mut().get_mut::<Animator>(root).unwrap().paused = false;
    step(&mut app, 0.125);
    let resumed = app.world().get::<Animator>(root).unwrap();
    assert_eq!(resumed.fade.as_ref().unwrap().elapsed, 0.25);
    assert_eq!(resumed.current.as_ref().unwrap().time, 0.25);
    assert!(
        bones
            .iter()
            .enumerate()
            .any(|(i, e)| *app.world().get::<Transform>(*e).unwrap() != frozen[i])
    );
}
