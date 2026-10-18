//! Existing customized characters: plain / enabled in place / enabled then disabled.
use aa_anim::facial::{
    FacialAnimator, FacialPlugin, FacialSequence, disable_character_face, enable_character_face,
};
use aa_anim::{
    AnimPlugin, Animator, CharacterDesc, ClipLibrary, GameDb, Gender, PartKind, Race,
    spawn_character,
};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::prelude::*;

#[derive(Component)]
struct Mode(usize);
#[derive(Resource)]
struct Done(bool);

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-anim in-place facial attachment"),
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
        .add_systems(Update, attach)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let db = GameDb::open().unwrap();
    let mut desc = CharacterDesc::default_for(&db, Race::Nuian, Gender::Male).unwrap();
    desc.hair = None;
    let mut assets = desc.resolve(&db).unwrap();
    assets.parts.retain(|p| p.kind == PartKind::Face);
    for i in 0..3 {
        let root = spawn_character(
            &mut commands,
            &mut cry,
            &clips,
            assets.clone(),
            Transform::from_xyz((i as f32 - 1.0) * 0.48, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
        )
        .unwrap();
        commands.entity(root).insert(Mode(i));
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 18000.0,
            ..default()
        },
        Transform::from_xyz(1.5, 3.0, 5.0).looking_at(Vec3::Y * 1.7, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Projection::Perspective(PerspectiveProjection {
            fov: 14.0f32.to_radians(),
            ..default()
        }),
        Transform::from_xyz(0.0, 1.72, 3.6).looking_at(Vec3::new(0.0, 1.68, 0.0), Vec3::Y),
    ));
    for (i, label) in ["PLAIN", "ENABLED / AUTHORED A", "DISABLED / ORIGINAL"]
        .into_iter()
        .enumerate()
    {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(i as f32 * 33.333),
                bottom: px(35),
                width: percent(33.333),
                ..default()
            },
            Text::new(label),
            TextLayout::justify(Justify::Center),
            TextFont {
                font_size: FontSize::Px(22.0),
                ..default()
            },
        ));
    }
}

fn attach(world: &mut World) {
    let elapsed = world.resource::<Time>().elapsed_secs();
    if let Some(done) = world.get_resource::<Done>() {
        if !done.0 && elapsed >= 5.0 {
            let root = world
                .query::<(Entity, &Mode)>()
                .iter(world)
                .find(|(_, mode)| mode.0 == 2)
                .unwrap()
                .0;
            assert!(disable_character_face(world, root));
            world.resource_mut::<Done>().0 = true;
            info!("in-place: disabled third character after its private facial meshes rendered");
        }
        return;
    }
    if elapsed < 3.0 {
        return;
    }
    let roots = world
        .query::<(Entity, &Mode)>()
        .iter(world)
        .map(|(entity, mode)| (entity, mode.0))
        .collect::<Vec<_>>();
    if roots.len() != 3 {
        return;
    }
    let sequence =
        FacialSequence::load("animations/nuian/male/facial/fist_ac_talk_11.fsq").unwrap();
    let time = sequence
        .channels
        .iter()
        .filter(|c| c.name == "a")
        .flat_map(|c| &c.splines)
        .flat_map(|s| &s.keys)
        .max_by(|a, b| a.value.total_cmp(&b.value))
        .unwrap()
        .time;
    for (root, mode) in roots {
        if mode != 0 {
            assert!(enable_character_face(world, root).unwrap());
            let mut face = world.get_mut::<FacialAnimator>(root).unwrap();
            face.play(sequence.clone(), false);
            face.time = time;
            face.paused = true;
            face.fade_seconds = 0.0;
        }
        let mut body = world.get_mut::<Animator>(root).unwrap();
        body.paused = true;
        body.random_idle = None;
        if let Some(clip) = &mut body.current {
            clip.time = 0.7;
        }
    }
    info!(
        "in-place: plain / authored A at {time}s / restored original; all body and material entities retained"
    );
    world.insert_resource(Done(false));
}
