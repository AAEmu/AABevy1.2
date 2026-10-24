use super::*;
use bevy::math::Mat3;
use serde_json::Value;

fn fixtures() -> Value {
    serde_json::from_str(include_str!("fixtures.json")).unwrap()
}
fn race(name: &str) -> Race {
    match name {
        "nuian" => Race::Nuian,
        "elf" => Race::Elf,
        "hariharan" => Race::Hariharan,
        "ferre" => Race::Ferre,
        _ => panic!("unknown fixture race"),
    }
}
fn gender(name: &str) -> Gender {
    match name {
        "male" => Gender::Male,
        "female" => Gender::Female,
        _ => panic!("unknown fixture gender"),
    }
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(
        v[0].as_f64().unwrap() as f32,
        v[1].as_f64().unwrap() as f32,
        v[2].as_f64().unwrap() as f32,
    )
}
fn native_record(hex: &str) -> [f32; 7] {
    std::array::from_fn(|i| {
        let bytes = std::array::from_fn(|j| {
            u8::from_str_radix(&hex[i * 8 + j * 2..i * 8 + j * 2 + 2], 16).unwrap()
        });
        f32::from_le_bytes(bytes)
    })
}
fn scene(data: &Value, name: &str, sex: &str) -> Scene {
    let source = data["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["race"] == name)
        .unwrap();
    Scene::parse(
        source["objects_xml"].as_str().unwrap(),
        race(name),
        gender(sex),
    )
    .unwrap()
}
fn assert_rotation(actual: Quat, matrix: &Value, context: &str) {
    let actual = Mat3::from_quat(actual).to_cols_array_2d();
    for row in 0..3 {
        for col in 0..3 {
            let expected = matrix[row][col].as_f64().unwrap() as f32;
            assert!(
                (actual[col][row] - expected).abs() < 3e-6,
                "{context}: {row},{col}: {} != {expected}",
                actual[col][row]
            );
        }
    }
}

#[test]
fn the_four_scene_imports_match_all_36_original_quat_t_records() {
    let data = fixtures();
    let mut records = 0;
    for source in data["scenes"].as_array().unwrap() {
        let r = source["race"].as_str().unwrap();
        for g in ["male", "female"] {
            let scene = scene(&data, r, g);
            let actor = native_record(source["actor_hex"].as_str().unwrap());
            let actual = [
                scene.actor.rotation.to_array().as_slice(),
                scene.actor.position.to_array().as_slice(),
            ]
            .concat();
            assert_eq!(
                actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                actor.map(f32::to_bits)
            );
            records += usize::from(g == "male");
            for (pose, raw) in scene
                .cameras
                .iter()
                .zip(source["camera_hex"][g].as_array().unwrap())
            {
                let expected = native_record(raw.as_str().unwrap());
                let actual = [
                    pose.rotation.to_array().as_slice(),
                    pose.position.to_array().as_slice(),
                ]
                .concat();
                assert_eq!(
                    actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    expected.map(f32::to_bits)
                );
                records += 1;
            }
        }
    }
    assert_eq!(records, 36);
}

#[test]
#[ignore = "reads the four original game_pak layer assets"]
fn real_pak_layer_imports_match_the_retained_original_service_records() {
    let data = fixtures();
    for source in data["scenes"].as_array().unwrap() {
        for g in ["male", "female"] {
            let loaded = Scene::load(race(source["race"].as_str().unwrap()), gender(g)).unwrap();
            let expected = native_record(source["actor_hex"].as_str().unwrap());
            assert_eq!(
                loaded.actor.position.to_array().map(f32::to_bits),
                expected[4..]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
                    .as_slice()
            );
            for (pose, raw) in loaded
                .cameras
                .iter()
                .zip(source["camera_hex"][g].as_array().unwrap())
            {
                let expected = native_record(raw.as_str().unwrap());
                assert_eq!(
                    pose.rotation.to_array().map(f32::to_bits),
                    expected[..4]
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>()
                        .as_slice()
                );
                assert_eq!(
                    pose.position.to_array().map(f32::to_bits),
                    expected[4..]
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>()
                        .as_slice()
                );
            }
        }
    }
}

#[test]
fn all_eight_shop_cameras_match_native_midpoint_offsets_and_axes() {
    let data = fixtures();
    let basis = aa_cry::CRY_TO_BEVY;
    for case in data["contexts"].as_array().unwrap() {
        let r = case["race"].as_str().unwrap();
        let g = case["gender"].as_str().unwrap();
        let scene = scene(&data, r, g);
        let mut pose = scene.shop_pose();
        assert_eq!(
            pose.position.to_array().map(f32::to_bits),
            vector(&case["midpoint"]["position_xyz"])
                .to_array()
                .map(f32::to_bits)
        );
        assert!(pose.adjust(vector(&case["offset"])));
        assert_eq!(
            pose.position.to_array().map(f32::to_bits),
            vector(&case["adjusted"]["position_xyz"])
                .to_array()
                .map(f32::to_bits)
        );
        let (camera, projection) = scene
            .camera(pose, UVec2::new(512, 512), Height::Disabled)
            .unwrap();
        assert_rotation(
            basis.inverse() * camera.rotation * basis,
            &case["camera_matrix"],
            &format!("{r}/{g}"),
        );
        assert!(
            (basis.inverse() * camera.translation - (pose.position - scene.actor.position))
                .length()
                < 1e-6
        );
        assert_eq!(projection.fov.to_bits(), 0x3f060a92);
        assert_eq!(
            (projection.aspect_ratio, projection.near, projection.far),
            (1., 0.1, 100.)
        );
        let (_, wide) = scene
            .camera(pose, UVec2::new(512, 256), Height::Disabled)
            .unwrap();
        assert_eq!(wide.aspect_ratio, 2.);
        let mut flipped = scene.clone();
        flipped.cameras[2].rotation = -flipped.cameras[2].rotation;
        assert!(
            flipped
                .shop_pose()
                .rotation
                .dot(scene.shop_pose().rotation)
                .abs()
                > 1. - 1e-6
        );
        flipped.cameras[3].rotation = -flipped.cameras[3].rotation;
        assert!(
            flipped
                .shop_pose()
                .rotation
                .dot(scene.shop_pose().rotation)
                .abs()
                > 1. - 1e-6
        );
    }
}

#[test]
fn actor_yaw_composes_in_local_z_including_noncommuting_native_controls() {
    let data = fixtures();
    let basis = aa_cry::CRY_TO_BEVY;
    for case in data["models"].as_array().unwrap() {
        let name = case["case"].as_str().unwrap();
        let (scene, yaw) = if name.starts_with("control_") {
            let mut s = scene(&data, "nuian", "male");
            s.actor.rotation = if name.ends_with('0') {
                Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
            } else {
                Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
            };
            (s, std::f32::consts::FRAC_PI_2)
        } else {
            let (r, yaw) = name.split_once("_model_frame_yaw_").unwrap();
            (
                scene(&data, r, "male"),
                yaw.parse::<f32>().unwrap().to_radians(),
            )
        };
        let actor = scene.actor_transform(yaw).unwrap();
        assert_eq!(actor.translation, Vec3::ZERO);
        assert_rotation(
            basis.inverse() * actor.rotation * basis,
            &case["matrix"],
            name,
        );
    }
}

#[test]
fn conditional_heel_tail_matches_13_native_scalar_results_without_a_second_model_lift() {
    let data = fixtures();
    let mut scene = scene(&data, "nuian", "female");
    scene.actor.position = Vec3::ZERO;
    for case in data["heights"].as_array().unwrap() {
        let height = if case["applied_gate"] == true && case["model_present"] == true {
            Height::Enabled {
                current: case["current_height"].as_f64().unwrap() as f32,
                reference: case["reference_height"].as_f64().unwrap() as f32,
            }
        } else {
            Height::Disabled
        };
        let pose = Pose {
            rotation: Quat::IDENTITY,
            position: Vec3::new(0., 0., case["base_camera_z"].as_f64().unwrap() as f32),
        };
        let (camera, _) = scene.camera(pose, UVec2::splat(512), height).unwrap();
        let expected = u32::from_str_radix(
            case["output_camera_z_bits"]
                .as_str()
                .unwrap()
                .trim_start_matches("0x"),
            16,
        )
        .unwrap();
        assert_eq!(
            pose.with_height(height).unwrap().position.z.to_bits(),
            expected,
            "{}",
            case["case"]
        );
        // Basis rotates about X; its floating multiplication has a bounded last-bit
        // effect. The native scalar addition itself remains exact before this map.
        assert!(
            (camera.translation.y - f32::from_bits(expected)).abs() < 4e-5,
            "{}",
            case["case"]
        );
        assert_eq!(scene.actor_transform(0.).unwrap().translation, Vec3::ZERO);
    }
    assert!(
        scene
            .camera(scene.shop_pose(), UVec2::splat(512), Height::Unknown)
            .is_none()
    );
}

#[test]
fn texture_caps_match_original_f32_order_including_rounding_discriminators() {
    let data = fixtures();
    for case in data["textures"].as_array().unwrap() {
        let pair = |key: &str| {
            IVec2::new(
                case[key][0].as_i64().unwrap() as i32,
                case[key][1].as_i64().unwrap() as i32,
            )
        };
        assert_eq!(
            texture_extent(pair("request"), pair("screen").as_uvec2()),
            Some(pair("admitted").as_uvec2())
        );
    }
    assert!(texture_extent(IVec2::new(0, 512), UVec2::splat(1024)).is_none());
    assert!(texture_extent(IVec2::splat(512), UVec2::ZERO).is_none());
    assert!(
        texture_extent(
            IVec2::new(i32::MAX, i32::MAX - 1),
            UVec2::new(i32::MAX as u32 - 1, i32::MAX as u32)
        )
        .is_none()
    );
}

#[test]
fn incomplete_or_ambiguous_scene_inputs_remain_unavailable() {
    let data = fixtures();
    let xml = data["scenes"][0]["objects_xml"].as_str().unwrap();
    assert!(
        Scene::parse(
            &xml.replace("04_customize_nuian_male_4", "missing"),
            Race::Nuian,
            Gender::Male
        )
        .is_err()
    );
    let duplicate = xml.replace("04_customize_nuian_male_3", "04_customize_nuian_male_4");
    assert!(Scene::parse(&duplicate, Race::Nuian, Gender::Male).is_err());
    let scene = scene(&data, "nuian", "male");
    let mut pose = scene.shop_pose();
    let before = pose.position;
    assert!(!pose.adjust(Vec3::splat(f32::NAN)));
    assert_eq!(pose.position, before);
    assert!(scene.actor_transform(f32::NAN).is_none());
}
