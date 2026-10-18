//! 32 independently posed identical heads through the public loaded-head factory.
//! Default: Nuian male. `--girl` selects the original girl01 native expression head.
use aa_anim::facial::{FacialPlugin, FacialSequence, prepare_loaded_face};
use aa_anim::{AnimPlugin, Animator, Cal, Cdf};
use aa_cry::scene::{CryAssets, CryPlugin, ModelOptions, SkeletonBinding, spawn_loaded};
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-anim facial crowd"),
            aa_dev::DevPlugin,
            CryPlugin,
            AnimPlugin,
            FacialPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.04, 0.045, 0.06)))
        .insert_resource(GlobalAmbientLight {
            brightness: 600.0,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, verify_instances)
        .run();
}

fn verify_instances(
    mut checked: Local<bool>,
    references: Query<&bevy::mesh::morph::MeshMorphWeights>,
    weights: Query<&bevy::mesh::morph::MorphWeights>,
) {
    if *checked || references.is_empty() {
        return;
    }
    let mut roots = std::collections::HashSet::new();
    let mut poses = std::collections::HashSet::new();
    for reference in &references {
        if let bevy::mesh::morph::MeshMorphWeights::Reference(root) = reference {
            roots.insert(*root);
            poses.insert(
                weights
                    .get(*root)
                    .unwrap()
                    .weights()
                    .iter()
                    .map(|w| w.to_bits())
                    .collect::<Vec<_>>(),
            );
        }
    }
    if poses.len() <= 1 {
        return;
    }
    assert_eq!(roots.len(), 32);
    info!(
        "crowd: {} subset references -> {} independent roots; {} distinct current poses",
        references.iter().count(),
        roots.len(),
        poses.len()
    );
    *checked = true;
}

fn setup(mut commands: Commands, mut cry: CryAssets) {
    let girl = std::env::args().any(|arg| arg == "--girl");
    let cdf = Cdf::load(if girl {
        "objects/characters/people/girl01/nude/girl01_base.cdf"
    } else {
        "objects/characters/nuian/male/nude/nu_m_base.cdf"
    })
    .unwrap();
    let head = if girl {
        cdf.attachment("face").unwrap().binding.clone()
    } else {
        "objects/characters/nuian/male/face/nu_m_face01.chr".into()
    };
    let cal = format!("{}.cal", cdf.model.trim_end_matches(".chr"));
    let rig = aa_anim::pose::shared_rig(&cdf.model).unwrap();
    let table = Cal::shared(&cal).unwrap();
    let sequence = FacialSequence::load(if girl {
        "animations/people/girl01/facial/fist_ac_talk_02.fsq"
    } else {
        &table.facial.get("fist_ac_talk_11").unwrap().path
    })
    .unwrap();
    let plain = cry.load_model(&head, &ModelOptions::default()).unwrap();
    let before = (cry.meshes.len(), cry.images.len());
    let start = std::time::Instant::now();
    let head_height = if girl { 1.15 } else { 1.72 };
    for i in 0..32 {
        let root = commands
            .spawn((
                Transform::from_xyz(
                    (i % 8) as f32 * 0.45 - 1.575,
                    (i / 8) as f32 * 0.44 - 0.66,
                    0.0,
                )
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                Visibility::default(),
            ))
            .id();
        let model_root = commands
            .spawn((
                Transform::from_rotation(aa_cry::CRY_TO_BEVY),
                Visibility::default(),
                ChildOf(root),
            ))
            .id();
        let bones = aa_anim::spawn_bones(&mut commands, model_root, &rig);
        let options = ModelOptions {
            skeleton: Some(SkeletonBinding {
                skeleton: rig.skeleton.clone(),
                joints: bones.clone(),
            }),
            cry_space: true,
            ..default()
        };
        let (loaded, mut face, weights) =
            prepare_loaded_face(&mut cry, &head, &cal, &plain).unwrap();
        let entity = spawn_loaded(&mut commands, &head, &loaded, &options);
        commands.entity(entity).insert(ChildOf(model_root));
        for eye in cdf.attachments.iter().filter(|a| {
            a.kind == aa_anim::cdf::AttachmentKind::Bone
                && a.name.starts_with("eye_")
                && !a.binding.is_empty()
        }) {
            let (bone, local) = eye.bone_binding(&rig.skeleton).unwrap();
            let options = ModelOptions {
                cry_space: true,
                ..default()
            };
            let loaded = cry.load_model(&eye.binding, &options).unwrap();
            let eye_entity = spawn_loaded(&mut commands, &eye.binding, &loaded, &options);
            commands.entity(eye_entity).insert((
                Transform::from_matrix(Mat4::from(local)),
                ChildOf(bones[bone]),
            ));
        }
        face.play(sequence.clone(), true);
        face.follow_body = false;
        face.fade_seconds = 0.0;
        face.time = i as f32 / 32.0 * sequence.end;
        let mut body = Animator::new(rig.clone(), bones);
        body.paused = true;
        commands.entity(root).insert((body, face, weights));
    }
    info!(
        "crowd: 32 prepared in {:?}; mesh/image assets {:?} -> {:?}",
        start.elapsed(),
        before,
        (cry.meshes.len(), cry.images.len())
    );
    commands.spawn((
        DirectionalLight {
            illuminance: 18000.0,
            ..default()
        },
        Transform::from_xyz(2.0, 4.0, 5.0).looking_at(Vec3::Y * head_height, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Projection::Perspective(PerspectiveProjection {
            fov: 18.0f32.to_radians(),
            ..default()
        }),
        Transform::from_xyz(0.0, head_height, 7.0)
            .looking_at(Vec3::new(0.0, head_height, 0.0), Vec3::Y),
    ));
}
