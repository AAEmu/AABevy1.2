//! Original authored FSQ close-ups. `--race nuian|elf|hariharan|ferre --gender m|f`;
//! `--live` runs the idle FSQ, `--bones` isolates the original Ferre male cry eye controllers.
//! `--head` previews the PC library's authored head_x effector at -1, 0 and +1 weight.
//! `--girl` previews the original girl01 npc_* targets and its own FXL.
//! `--chef` previews the NPC chef's original native face01 speech targets.
//! `--chef-items` fills the actor CDF's declared slots with explicit original body items.
use aa_anim::facial::{ExpressionLibrary, FacialAnimator, FacialPlugin, FacialSequence};
use aa_anim::{AnimPlugin, Animator, CharacterDesc, ClipLibrary, GameDb, Gender, Race, spawn_character_with_facial};
use aa_cry::scene::{CryAssets, CryPlugin, ModelOptions, SkeletonBinding, spawn_loaded};
use bevy::prelude::*;

fn arg(name: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-anim authored facial sequences"),
            aa_dev::DevPlugin,
            CryPlugin,
            AnimPlugin,
            FacialPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.75, 0.8, 0.9),
            brightness: 600.0,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, freeze)
        .run();
}

#[derive(Component)]
struct Preview {
    path: String,
    time: f32,
    library: Option<String>,
    head_weight: Option<f32>,
}

// Mirrors the client's CDF skin path: load against shared body joints, prepare before
// spawn_loaded, then put the returned face and weights beside the body Animator.
fn spawn_npc(commands: &mut Commands, cry: &mut CryAssets, transform: Transform) -> Entity {
    let with_items = std::env::args().any(|a| a == "--chef-items");
    let girl = std::env::args().any(|a| a == "--girl");
    let mut cdf = aa_anim::Cdf::load(if girl { "objects/characters/people/girl01/nude/girl01_base.cdf" } else if with_items { "objects/characters/people/npc_chef01/nude/npc_chef01.cdf" } else { "objects/characters/people/npc_chef01/nude/npc_chef01_base.cdf" }).unwrap();
    if with_items {
        let db = GameDb::open().unwrap();
        for item in [14478, 14475, 14479] { assert!(cdf.bind_body_part(&db.body_part(item).unwrap())); }
    }
    let head_path = cdf.attachment("face").unwrap().binding.as_str();
    let body_cal = if girl { "objects/characters/people/girl01/nude/girl01_base.cal" } else { "objects/characters/people/npc_chef01/nude/npc_chef01_base.cal" };
    let rig = aa_anim::pose::shared_rig(&cdf.model).unwrap();
    let root = commands.spawn((transform, Visibility::default())).id();
    let model_root = commands.spawn((Transform::from_rotation(aa_cry::CRY_TO_BEVY), Visibility::default(), ChildOf(root))).id();
    let bones = aa_anim::spawn_bones(commands, model_root, &rig);
    let options = ModelOptions {
        skeleton: Some(SkeletonBinding { skeleton: rig.skeleton.clone(), joints: bones.clone() }),
        cry_space: true,
        ..default()
    };
    let plain = cry.load_model(head_path, &options).unwrap();
    let (loaded, face, weights) = aa_anim::facial::prepare_loaded_face(cry, head_path, body_cal, &plain).unwrap();
    assert_eq!(face.targets.len(), if girl { 24 } else { 23 });
    let head = spawn_loaded(commands, head_path, &loaded, &options);
    commands.entity(head).insert(ChildOf(model_root));
    if with_items {
        for attachment in &cdf.attachments {
            if attachment.kind != aa_anim::cdf::AttachmentKind::Skin || attachment.name == "face" || attachment.binding.is_empty() || attachment.flags & 1 != 0 { continue; }
            let options = ModelOptions { material: attachment.material.clone(), ..options.clone() };
            let loaded = cry.load_model(&attachment.binding, &options).unwrap();
            let part = spawn_loaded(commands, &attachment.binding, &loaded, &options);
            commands.entity(part).insert(ChildOf(model_root));
        }
    }
    for attachment in &cdf.attachments {
        if attachment.kind != aa_anim::cdf::AttachmentKind::Bone || attachment.binding.is_empty() || attachment.flags & 1 != 0 { continue; }
        let (bone, local) = attachment.bone_binding(&rig.skeleton).unwrap();
        let options = ModelOptions { material: attachment.material.clone(), cry_space: true, ..default() };
        let loaded = cry.load_model(&attachment.binding, &options).unwrap();
        let part = spawn_loaded(commands, &attachment.binding, &loaded, &options);
        commands.entity(part).insert((Transform::from_matrix(Mat4::from(local)), ChildOf(bones[bone])));
    }
    commands.entity(root).insert((Animator::new(rig, bones), face, weights));
    root
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let chef = std::env::args().any(|a| a == "--chef" || a == "--chef-items");
    let girl = std::env::args().any(|a| a == "--girl");
    let npc = chef || girl;
    let bones = std::env::args().any(|a| a == "--bones");
    let head_preview = std::env::args().any(|a| a == "--head");
    let race = match arg("--race").as_deref() {
        Some("elf") => Race::Elf,
        Some("hariharan") => Race::Hariharan,
        Some("ferre") => Race::Ferre,
        _ => Race::Nuian,
    };
    let gender = if arg("--gender").is_some_and(|s| s.starts_with('f')) {
        Gender::Female
    } else {
        Gender::Male
    };
    let (race, gender) = if head_preview {
        (Race::Nuian, Gender::Male)
    } else if bones {
        (Race::Ferre, Gender::Male)
    } else {
        (race, gender)
    };
    let db = GameDb::open().expect("game database");
    let dir = if girl { "animations/people/girl01/facial".into() } else if chef { "animations/people/chef_01/facial".into() } else { format!("animations/{}/{}/facial", race.folder(), gender.folder()) };
    let path_for = |name: &str| {
        let candidate = format!("{dir}/{name}.fsq");
        if aa_pak::Pak::global().exists(&candidate) {
            candidate
        } else {
            format!("animations/nuian/{}/facial/{name}.fsq", gender.folder())
        }
    };
    let peak = |path: &str, channel: &str| {
        let sequence = FacialSequence::load(path).ok()?;
        sequence
            .channels
            .iter()
            .filter(|c| c.name.eq_ignore_ascii_case(channel))
            .flat_map(|c| c.splines.iter())
            .flat_map(|s| &s.keys)
            .filter(|k| k.value > 0.2)
            .max_by(|a, b| a.value.total_cmp(&b.value))
            .map(|k| (k.value, k.time))
    };
    let idle = path_for(if girl { "fist_ac_talk_01" } else if chef { "fist_ac_talk_11" } else { "fist_ac_stage_idle" });
    let blink_time = if npc { 0.0 } else { peak(&idle, "L_close_suprise").expect("authored blink keys").1 };
    // Female default.fsq is neutral; use the strongest authored A key in these real sequences.
    let (speech_path, speech_time) = if girl { vec!["fist_ac_talk_01", "fist_ac_talk_02"] } else if chef { vec!["fist_ac_talk_11"] } else { vec!["default", "fist_ac_talk_11", "npc_question"] }
        .into_iter()
        .filter_map(|name| {
            let path = path_for(name);
            peak(&path, "a").map(|(weight, time)| (path, time, weight))
        })
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(path, time, _)| (path, time))
        .expect("authored speech keys");
    let mut samples = [
        ("OPEN / IDLE", idle.clone(), 0.7),
        ("BLINK / IDLE", idle, blink_time),
        ("SPEECH / A", speech_path, speech_time),
    ];
    if npc {
        let path = if girl { samples[2].1.clone() } else { path_for("fist_ac_talk_11") };
        let end = if girl { FacialSequence::load(&path).unwrap().end } else { 3.0 };
        samples = [("NPC / NEUTRAL", path.clone(), 0.0), ("NPC / AUTHORED A", path.clone(), speech_time), (if girl { "NPC / AUTHORED END" } else { "NPC / RETURN" }, path, end)];
    }
    if bones {
        let path = "animations/ferre/male/facial/fist_em_cry.fsq";
        samples = [
            ("EYES / NEUTRAL", path.into(), 0.0),
            ("EYES / CRY", path.into(), 0.239096),
            ("EYES / RETURN", path.into(), 1.67368),
        ];
    }
    if head_preview {
        samples = [
            ("FXL HEAD X / -1", String::new(), -1.0),
            ("FXL HEAD X / 0", String::new(), 0.0),
            ("FXL HEAD X / +1", String::new(), 1.0),
        ];
    }
    let live = !bones && !head_preview && std::env::args().any(|a| a == "--live");
    for (i, (_, path, time)) in samples.iter().enumerate() {
        let mut desc = CharacterDesc::from_preset(&db, race, gender, 0).unwrap();
        desc.hair = None;
        let transform = Transform::from_xyz((i as f32 - 1.0) * 0.48, 0.0, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::PI));
        let root = if npc { spawn_npc(&mut commands, &mut cry, transform) } else {
            spawn_character_with_facial(&mut commands, &mut cry, &clips, desc.resolve(&db).unwrap(), transform).unwrap()
        };
        if !live {
            commands.entity(root).insert(Preview {
                path: path.clone(),
                time: *time,
                library: (bones && !head_preview).then(|| "animations/ferre/male/facial/ferre_m.fxl".into()),
                head_weight: head_preview.then_some(*time),
            });
        }
    }
    let label = |index: usize| {
        if live {
            "LIVE / BODY FSQ".into()
        } else if head_preview {
            samples[index].0.into()
        } else {
            format!("{} {:.2}s", samples[index].0, samples[index].2)
        }
    };
    let head = if girl { 1.15 } else if chef { 1.55 } else { match (race, gender) {
        (Race::Nuian | Race::Elf, Gender::Male) => 1.72,
        (Race::Nuian | Race::Elf, _) => 1.62,
        (Race::Hariharan, Gender::Male) => 1.58,
        (Race::Hariharan, _) => 1.47,
        (Race::Ferre, Gender::Male) => 1.9,
        (Race::Ferre, _) => 1.7,
    } };
    for (at, lux, color) in [
        (Vec3::new(1.5, 3.0, 5.0), 18000.0, Color::srgb(1.0, 0.96, 0.9)),
        (Vec3::new(-4.0, 2.0, 3.0), 5000.0, Color::srgb(0.6, 0.7, 1.0)),
        (Vec3::new(0.0, 3.0, -5.0), 6000.0, Color::srgb(1.0, 0.9, 0.8)),
    ] {
        commands.spawn((
            DirectionalLight {
                illuminance: lux,
                color,
                ..default()
            },
            Transform::from_translation(at).looking_at(Vec3::new(0.0, 1.5, 0.0), Vec3::Y),
        ));
    }
    let mut camera = commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Projection::Perspective(PerspectiveProjection {
            fov: 14.0f32.to_radians(),
            ..default()
        }),
        Transform::from_xyz(0.0, head, 3.6).looking_at(Vec3::new(0.0, head - 0.04, 0.0), Vec3::Y),
    ));
    if let Some(env) = aa_cry::stage::environment_light(&mut cry, aa_cry::stage::DEFAULT_ENVIRONMENT, 3000.0) {
        camera.insert(env);
    }
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            bottom: px(35),
            width: percent(100),
            justify_content: JustifyContent::SpaceEvenly,
            ..default()
        },
        children![
            (
                Text::new(label(0)),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                }
            ),
            (
                Text::new(label(1)),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                }
            ),
            (
                Text::new(label(2)),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                }
            ),
        ],
    ));
}

fn freeze(mut commands: Commands, mut faces: Query<(Entity, &Preview, &mut FacialAnimator, &mut Animator)>) {
    for (entity, preview, mut face, mut body) in &mut faces {
        if let Some(path) = &preview.library {
            face.library = ExpressionLibrary::load(path).unwrap();
        }
        let sequence = if let Some(weight) = preview.head_weight {
            // An explicit effector diagnostic, not a claim that an original FSQ moves the head.
            std::sync::Arc::new(FacialSequence::parse(&format!(
                r#"<FacialSequence EndTime="1"><Channel Name="head_x"><Spline Keys="0:{weight}:0,"/></Channel></FacialSequence>"#
            )).unwrap())
        } else {
            FacialSequence::load(&preview.path).unwrap()
        };
        face.play(sequence, true);
        face.time = if preview.head_weight.is_some() { 0.0 } else { preview.time };
        face.paused = true;
        face.fade_seconds = 0.0;
        body.paused = true;
        body.random_idle = None;
        if let Some(playing) = &mut body.current {
            playing.time = 0.7;
        }
        info!(
            "aa-anim facial: {} at {}s; {} targets",
            preview.path,
            preview.time,
            face.targets.len()
        );
        commands.entity(entity).remove::<Preview>();
    }
}
