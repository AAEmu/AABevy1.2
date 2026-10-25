//! Original non-mount ship clips through CAL -> ClipLibrary -> Animator.
//! `cargo run -p aa-anim --example legacy_ship -- --shot shots/aa-anim/legacy_ship.png --shot-after 8`
//! Two authored poses per row: 0x827 helm (top), 0x828 anchor (bottom).

use aa_anim::{AnimPlugin, Animator, Cal, ClipLibrary, SampleOptions, pose::shared_rig, spawn_bones};
use aa_cry::scene::{CryAssets, CryPlugin, ModelOptions, SkeletonBinding, spawn_loaded};
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-anim original legacy ship controllers"),
            aa_dev::DevPlugin,
            CryPlugin,
            AnimPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.035, 0.045, 0.06)))
        .insert_resource(GlobalAmbientLight {
            brightness: 900.0,
            ..default()
        })
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let folder = "objects/env/06_unit/01_ship/galleon";
    for (row, name, action, controller, times) in [
        (0, "galleon_helms", "fist_npc_helmsman_relaxed", 0xcb91ae76, [0.0, 14.5]),
        (1, "galleon_anchor", "spread", 0x78561a24, [0.0, 4.0]),
    ] {
        let model = format!("{folder}/{name}.chr");
        let cal = Cal::load(&format!("{folder}/{name}.cal")).expect("original CAL");
        let entry = cal.animation(action).expect("authored action");
        let rig = shared_rig(&model).expect("original skeleton");
        let bound = clips
            .bound(
                &rig,
                &entry.path,
                SampleOptions {
                    lock_root_motion: false,
                    ..default()
                },
            )
            .expect("legacy clip");
        assert_eq!(bound.clip.skipped_controllers, 0);
        assert!(bound.clip.track(controller).is_some());
        let bone = rig.bone_by_controller(controller).expect("legacy controller bone");
        info!(
            "legacy ship: {name} {action}: {} tracks, {} bound, legacy bone {}",
            bound.clip.tracks.len(),
            bound.binding.pairs.len(),
            rig.skeleton.bones[bone].name
        );
        for (column, seconds) in times.into_iter().enumerate() {
            let root = commands.spawn((Transform::default(), Visibility::default())).id();
            let model_root = commands
                .spawn((
                    Transform::from_rotation(aa_cry::CRY_TO_BEVY),
                    Visibility::default(),
                    ChildOf(root),
                ))
                .id();
            let bones = spawn_bones(&mut commands, model_root, &rig);
            let options = ModelOptions {
                skeleton: Some(SkeletonBinding {
                    skeleton: rig.skeleton.clone(),
                    joints: bones.clone(),
                }),
                cry_space: true,
                ..default()
            };
            let loaded = cry.load_model(&model, &options).expect("original mesh");
            let part = spawn_loaded(&mut commands, &model, &loaded, &options);
            commands.entity(part).insert(ChildOf(model_root));
            let mut animator = Animator::new(rig.clone(), bones);
            animator.play(bound.clone(), false, 0.0);
            animator.update(seconds);
            animator.paused = true;
            // Center the original bone position in each panel. Both anchor
            // poses use the same display scale to fit the longer assembly.
            let center = aa_cry::CRY_TO_BEVY * animator.pose().model_space(&rig)[bone].1;
            let scale = if row == 0 { 1.0 } else { 0.4 };
            let placement = Vec3::new((column as f32 - 0.5) * 3.8, 2.5 - row as f32 * 3.0, 0.0) - center * scale;
            commands.entity(root).insert((
                Transform::from_translation(placement).with_scale(Vec3::splat(scale)),
                animator,
            ));
            commands.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: percent(column as f32 * 50.0),
                    top: percent(row as f32 * 50.0 + 3.0),
                    width: percent(50.0),
                    ..default()
                },
                Text::new(format!(
                    "{} / {seconds:.1}s",
                    if row == 0 { "0x827 HELM" } else { "0x828 ANCHOR" }
                )),
                TextLayout::justify(Justify::Center),
                TextFont {
                    font_size: FontSize::Px(24.0),
                    ..default()
                },
            ));
        }
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 18000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 7.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical { viewport_height: 8.0 },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, 1.2, 15.0).looking_at(Vec3::new(0.0, 1.2, 0.0), Vec3::Y),
    ));
}
