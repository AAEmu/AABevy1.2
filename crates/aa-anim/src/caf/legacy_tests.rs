use super::{Clip, Track, legacy, tests::build_caf};
use aa_cry::chunk::ty;
use glam::{Quat, Vec3};

#[path = "legacy_goldens.rs"]
mod goldens;

fn payload(id: u32, keys: &[(i32, [f32; 3], [f32; 3])]) -> Vec<u8> {
    let mut out = (keys.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&id.to_le_bytes());
    for (ticks, pos, log) in keys {
        out.extend_from_slice(&ticks.to_le_bytes());
        for value in pos.iter().chain(log) {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

fn rotation_close(actual: Quat, expected: [f32; 4]) {
    let expected = Quat::from_array(expected);
    let sign = if actual.dot(expected) < 0.0 { -1.0 } else { 1.0 };
    assert!(
        (actual - expected * sign).to_array().iter().all(|v| v.abs() < 2e-6),
        "{actual:?} vs {expected:?}"
    );
}

fn native_sample(track: &Track, t: f32, expected: [f32; 7]) {
    rotation_close(track.sample_rot(t).unwrap(), expected[..4].try_into().unwrap());
    let pos = track.sample_pos(t).unwrap();
    let expected = Vec3::from_array(expected[4..].try_into().unwrap());
    assert!((pos - expected).abs().max_element() < 3e-6, "{pos:?} vs {expected:?}");
}

#[test]
fn pqlog_matches_native_signed_tick_and_cm_import() {
    // Literal original-instruction output, including signed truncation toward
    // zero. These counts exercise native scalar, unrolled and tail paths.
    for version in [0x827, 0x828] {
        for (ticks, native_frames) in [
            ([-321, -159, 0, 321, 799], [-2, 0, 0, 2, 4]),
            ([-321, -159, 161, 321, 799], [-2, 0, 1, 2, 4]),
        ] {
            for n in [1, 4, 5] {
                let keys: Vec<_> = ticks[..n]
                    .iter()
                    .map(|&t| (t, [100.25, -50.5, 0.125], [0.1, 0.2, -0.3]))
                    .collect();
                let file = build_caf(&[(ty::CONTROLLER, version, 7, payload(42, &keys))]);
                let clip = Clip::parse(&file).unwrap();
                assert_eq!(clip.skipped_controllers, 0);
                let track = clip.track(42).unwrap();
                assert_eq!(track.rot.len(), n);
                assert_eq!(
                    track.rot_times,
                    native_frames[..n].iter().map(|&f| f as f32 / 30.0).collect::<Vec<_>>()
                );
                assert_eq!(track.rot_times, track.pos_times);
                for &p in &track.pos {
                    assert_eq!(
                        p,
                        Vec3::new(
                            f32::from_bits(0x3f8051eb),
                            f32::from_bits(0xbf0147ae),
                            f32::from_bits(0x3aa3d70a)
                        )
                    );
                }
            }
        }
    }
}

#[test]
fn all_original_ship_cafs_decode_both_legacy_versions() {
    if !aa_cry::pak_available() {
        return;
    }
    let pak = aa_pak::Pak::global();
    let mut versions = [0usize; 3];
    let mut count = 0;
    for path in pak
        .names()
        .filter(|p| p.starts_with("game/animations/env/ship/") && p.ends_with(".caf"))
    {
        let bytes = aa_cry::read(path).unwrap();
        let file = aa_cry::chunk::ChunkFile::parse(&bytes).unwrap();
        for c in file.of_type(ty::CONTROLLER) {
            match c.version {
                0x827 => versions[0] += 1,
                0x828 => versions[1] += 1,
                0x829 => versions[2] += 1,
                v => panic!("{path}: unexpected controller {v:#x}"),
            }
        }
        let clip = Clip::parse(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(clip.skipped_controllers, 0, "{path}");
        assert!(!clip.tracks.is_empty(), "{path}");
        count += 1;
    }
    assert_eq!((count, versions), (98, [2, 95, 2860]));
}

#[test]
fn pqlog_sampling_matches_executed_native_branches_and_near_zero() {
    for &(case, frame, expected) in goldens::SAMPLES {
        let (a, b) = goldens::LOGS[case];
        let track = legacy::decode(
            &payload(1, &[(0, [1.0, 2.0, 3.0], a), (1600, [4.0, -5.0, 6.0], b)]),
            30.0,
            0.0,
        )
        .unwrap();
        // Native synthetic positions were already in metres; the importer
        // converts cm, so compare against the corresponding native position.
        let mut expected = expected;
        for p in &mut expected[4..] {
            *p *= 0.01;
        }
        native_sample(&track, frame / 30.0, expected);
    }
}

#[test]
fn pqlog_duplicate_frames_match_native_search_for_position_and_rotation() {
    // Counts on either side of binary-search thresholds, every duplicate group,
    // exact times and +/-0.0001 frames; first/last groups include clamping.
    for &(count, frame, expected) in goldens::DUPLICATE_SAMPLES {
        let track = legacy::decode(&payload(1, &goldens::DUPLICATE_KEYS[..count]), 30.0, 0.0).unwrap();
        native_sample(&track, frame / 30.0, expected);
    }
}

#[test]
fn pqlog_malformed_data_is_rejected_without_panics() {
    let valid = payload(7, &[(0, [1.0, 2.0, 3.0], [0.0; 3]), (160, [4.0, 5.0, 6.0], [0.1; 3])]);
    for length in 0..valid.len() {
        assert!(
            legacy::decode(&valid[..length], 30.0, 0.0).is_err(),
            "truncated at {length}"
        );
    }
    let mut extra = valid.clone();
    extra.push(0);
    assert!(legacy::decode(&extra, 30.0, 0.0).is_err());
    let mut oversized = valid.clone();
    oversized[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(legacy::decode(&oversized, 30.0, 0.0).is_err());
    assert!(legacy::decode(&payload(7, &[]), 30.0, 0.0).is_err());
    for ticks in [-160, -320] {
        assert!(
            legacy::decode(
                &payload(7, &[(0, [0.0; 3], [0.0; 3]), (ticks, [0.0; 3], [0.0; 3])]),
                30.0,
                0.0
            )
            .is_err()
        );
    }
    assert!(
        legacy::decode(
            &payload(7, &[(0, [0.0; 3], [0.0; 3]), (100, [0.0; 3], [0.0; 3])]),
            30.0,
            0.0
        )
        .is_ok()
    );
    for (at, value) in [
        (12, f32::NAN),
        (16, f32::INFINITY),
        (24, f32::NEG_INFINITY),
        (28, f32::MAX),
    ] {
        let mut invalid = valid.clone();
        invalid[at..at + 4].copy_from_slice(&value.to_le_bytes());
        assert!(legacy::decode(&invalid, 30.0, 0.0).is_err());
    }
    // Unknown controller versions retain the existing explicit skip counter.
    let file = build_caf(&[(ty::CONTROLLER, 0x826, 1, vec![]), (ty::CONTROLLER, 0x828, 2, valid)]);
    let clip = Clip::parse(&file).unwrap();
    assert_eq!((clip.tracks.len(), clip.skipped_controllers), (1, 1));
}

#[test]
fn original_ship_cal_cdf_clips_bind_and_drive_the_existing_animator() {
    if !aa_cry::pak_available() {
        return;
    }
    use crate::{Animator, Cal, Cdf, ClipLibrary, SampleOptions, pose::shared_rig};
    let clips = ClipLibrary::default();
    let cdf = Cdf::load("objects/env/06_unit/01_ship/galleon/galleon_anchor.cdf").unwrap();
    assert_eq!(cdf.model, "objects/env/06_unit/01_ship/galleon/galleon_anchor.chr");
    for (asset, cal_path, action, model) in [
        (0, "galleon_anchor.cal", "spread", cdf.model.as_str()),
        (
            1,
            "galleon_sail0.cal",
            "fold",
            "objects/env/06_unit/01_ship/galleon/galleon_sail0.chr",
        ),
        (
            2,
            "galleon_helms.cal",
            "fist_npc_helmsman_combat",
            "objects/env/06_unit/01_ship/galleon/galleon_helms.chr",
        ),
        (
            3,
            "galleon_helms.cal",
            "fist_npc_helmsman_relaxed",
            "objects/env/06_unit/01_ship/galleon/galleon_helms.chr",
        ),
    ] {
        let cal = Cal::load(&format!("objects/env/06_unit/01_ship/galleon/{cal_path}")).unwrap();
        let entry = cal.animation(action).unwrap();
        assert_eq!(entry.path, goldens::ASSET_PATHS[asset]);
        let rig = shared_rig(model).unwrap();
        let bound = clips
            .bound(
                &rig,
                &entry.path,
                SampleOptions {
                    lock_root_motion: false,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(bound.clip.skipped_controllers, 0);
        let controller = goldens::CONTROLLERS[asset];
        let bone = rig.bone_by_controller(controller).unwrap();
        let track = bound.clip.track(controller).unwrap();
        assert_eq!(track.rot.len(), goldens::KEY_COUNTS[asset]);
        assert!(
            bound
                .binding
                .pairs
                .iter()
                .any(|&(b, t)| b as usize == bone && bound.clip.tracks[t as usize].controller == controller)
        );
        let mut animator = Animator::new(rig.clone(), Vec::new());
        for &(which, seconds, expected) in goldens::ASSET_SAMPLES {
            if which != asset {
                continue;
            }
            native_sample(track, seconds, expected);
            if seconds < 0.0 || seconds > bound.clip.duration {
                continue;
            }
            animator.play(bound.clone(), false, 0.0);
            animator.update(seconds);
            rotation_close(animator.pose().rot[bone], expected[..4].try_into().unwrap());
            assert!(
                (animator.pose().pos[bone] - Vec3::from_array(expected[4..].try_into().unwrap()))
                    .abs()
                    .max_element()
                    < 3e-6
            );
        }
        // The 0x828 anchor/sail root tracks are nearly constant and may equal
        // bind pose. The relaxed 0x827 helm controller turns the wheel; prove
        // playback reaches a changing pose instead of just loading more keys.
        if asset == 3 {
            let (key, _) = track
                .rot
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.dot(track.rot[0]).abs().total_cmp(&b.dot(track.rot[0]).abs()))
                .unwrap();
            animator.play(bound.clone(), false, 0.0);
            animator.update(track.rot_times[key]);
            assert!(
                animator.pose().rot[bone].dot(track.rot[0]).abs() < 0.9999,
                "{action}: wheel did not turn"
            );
            // Exercise the real mesh weights too: the newly decoded pose must
            // move original vertices, not merely an unused skeleton joint.
            let mesh = aa_cry::Model::load(model).unwrap();
            let posed = animator.pose().model_space(&rig);
            let mut moved = 0.0f32;
            for part in &mesh.meshes {
                let Some(skin) = &part.skin else { continue };
                for (v, (joints, weights)) in part.positions.iter().zip(skin.joints.iter().zip(&skin.weights)) {
                    let mut p = Vec3::ZERO;
                    for (&joint, &weight) in joints.iter().zip(weights) {
                        let joint = joint as usize;
                        let local = rig.skeleton.bones[joint].inverse_bind().transform_point3(*v);
                        p += (posed[joint].0 * local + posed[joint].1) * weight;
                    }
                    moved = moved.max(p.distance(*v));
                }
            }
            assert!(moved > 0.1, "{action}: original mesh vertices did not move ({moved})");
        }
    }
}
