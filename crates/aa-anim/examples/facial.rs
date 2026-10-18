//! Original authored FSQ close-ups: idle open, the stage-idle blink peak, and a speech A peak. `--race nuian|elf|hariharan|ferre --gender m|f`; `--live` runs the idle FSQ.
use aa_anim::facial::{FacialAnimator, FacialPlugin, FacialSequence};
use aa_anim::{AnimPlugin, Animator, CharacterDesc, ClipLibrary, GameDb, Gender, Race, spawn_character_with_facial};
use aa_cry::scene::{CryAssets, CryPlugin};
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
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
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
    let db = GameDb::open().expect("game database");
    let dir = format!("animations/{}/{}/facial", race.folder(), gender.folder());
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
    let idle = path_for("fist_ac_stage_idle");
    let blink_time = peak(&idle, "L_close_suprise").expect("authored blink keys").1;
    // Female default.fsq is neutral; use the strongest authored A key in these real sequences.
    let (speech_path, speech_time) = ["default", "fist_ac_talk_11", "npc_question"]
        .into_iter()
        .filter_map(|name| {
            let path = path_for(name);
            peak(&path, "a").map(|(weight, time)| (path, time, weight))
        })
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(path, time, _)| (path, time))
        .expect("authored speech keys");
    let samples = [
        ("OPEN / IDLE", idle.clone(), 0.7),
        ("BLINK / IDLE", idle, blink_time),
        ("SPEECH / A", speech_path, speech_time),
    ];
    let live = std::env::args().any(|a| a == "--live");
    for (i, (_, path, time)) in samples.iter().enumerate() {
        let mut desc = CharacterDesc::from_preset(&db, race, gender, 0).unwrap();
        desc.hair = None;
        let transform = Transform::from_xyz((i as f32 - 1.0) * 0.48, 0.0, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::PI));
        let root = spawn_character_with_facial(&mut commands, &mut cry, &clips, desc.resolve(&db).unwrap(), transform).unwrap();
        if !live {
            commands.entity(root).insert(Preview {
                path: path.clone(),
                time: *time,
            });
        }
    }
    let label = |index: usize| {
        if live {
            "LIVE / BODY FSQ".into()
        } else {
            format!("{} {:.2}s", samples[index].0, samples[index].2)
        }
    };
    let head = match (race, gender) {
        (Race::Nuian | Race::Elf, Gender::Male) => 1.72,
        (Race::Nuian | Race::Elf, _) => 1.62,
        (Race::Hariharan, Gender::Male) => 1.58,
        (Race::Hariharan, _) => 1.47,
        (Race::Ferre, Gender::Male) => 1.9,
        (Race::Ferre, _) => 1.7,
    };
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
        face.play(FacialSequence::load(&preview.path).unwrap(), true);
        face.time = preview.time;
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
