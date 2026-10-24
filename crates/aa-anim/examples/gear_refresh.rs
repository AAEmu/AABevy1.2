//! Original gear on existing actors: normal hair / helmet alternate / hood Cut.
//! At 3s the middle actor gets a helmet; the right gets a hood and authored heels.
//! At 5s the right's late facial overlay is disabled, retaining its new cut face.
use aa_anim::character::replace_character_parts;
use aa_anim::facial::{
    FacialAnimator, FacialPlugin, FacialSequence, disable_character_face, enable_character_face,
};
use aa_anim::{
    AnimPlugin, Animator, Character, CharacterDesc, ClipLibrary, Costume, GameDb, Gender, PartKind,
    Race, spawn_character,
};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::prelude::*;

#[derive(Component)]
struct Figure(usize);
#[derive(Resource, Default)]
struct Step(u8);

fn assets(items: &[u32]) -> aa_anim::CharacterAssets {
    let db = GameDb::open().unwrap();
    let mut desc = CharacterDesc::default_for(&db, Race::Nuian, Gender::Female).unwrap();
    desc.hair = Some(24024);
    desc.hair_color = None;
    desc.costume = Costume::Items(items.to_vec());
    let detail = items
        .iter()
        .filter_map(|&id| db.item_visuals(id, 11).ok())
        .flatten()
        .find(|v| matches!((v.detail >> 13) & 7, 3 | 7))
        .map(|v| v.detail);
    desc.resolve_with_headgear(&db, detail).unwrap()
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-anim existing actor gear refresh"),
            aa_dev::DevPlugin,
            CryPlugin,
            AnimPlugin,
            FacialPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.055, 0.06, 0.07)))
        .insert_resource(GlobalAmbientLight {
            brightness: 650.0,
            ..default()
        })
        .init_resource::<Step>()
        .add_systems(Startup, setup)
        .add_systems(Update, refresh)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    for i in 0..3 {
        let root = spawn_character(
            &mut commands,
            &mut cry,
            &clips,
            assets(&[]),
            Transform::from_xyz((i as f32 - 1.0) * 0.67, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
        )
        .unwrap();
        commands.entity(root).insert(Figure(i));
    }
    for (position, lux) in [
        (Vec3::new(2.0, 3.0, 4.0), 18000.0),
        (Vec3::new(-3.0, 2.0, 1.0), 5000.0),
    ] {
        commands.spawn((
            DirectionalLight {
                illuminance: lux,
                ..default()
            },
            Transform::from_translation(position).looking_at(Vec3::Y * 1.5, Vec3::Y),
        ));
    }
    let args = std::env::args().collect::<Vec<_>>();
    let view = args
        .iter()
        .position(|a| a == "--view")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    let (height, fov) = match view {
        Some("body") => (0.95, 26.0f32),
        Some("feet") => (0.18, 14.0f32),
        _ => (1.58, 14.0f32),
    };
    let floor = cry.meshes.add(Plane3d::default().mesh().size(4.0, 3.0));
    let material = cry.materials.add(StandardMaterial {
        base_color: Color::srgb(0.13, 0.14, 0.16),
        ..default()
    });
    commands.spawn((
        Mesh3d(floor),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, -0.004, 0.0),
    ));
    commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Projection::Perspective(PerspectiveProjection {
            fov: fov.to_radians(),
            ..default()
        }),
        Transform::from_xyz(0.0, height, 4.8).looking_at(Vec3::new(0.0, height, 0.0), Vec3::Y),
    ));
    for (i, label) in [
        "UNCHANGED / NORMAL HAIR",
        "HELMET / ALTERNATE HAIR AT 3s",
        "HOOD CUT + HEELS AT 3s / DISABLE AT 5s",
    ]
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
                font_size: FontSize::Px(20.0),
                ..default()
            },
        ));
    }
}

fn refresh(world: &mut World) {
    let elapsed = world.resource::<Time>().elapsed_secs();
    let step = world.resource::<Step>().0;
    if step == 0 && elapsed >= 1.0 {
        let roots = world
            .query::<(Entity, &Figure)>()
            .iter(world)
            .map(|(e, _)| e)
            .collect::<Vec<_>>();
        let sequence =
            FacialSequence::load("animations/nuian/female/facial/fist_ac_talk_11.fsq").unwrap();
        let time = sequence
            .channels
            .iter()
            .filter(|c| c.name == "a")
            .flat_map(|c| &c.splines)
            .flat_map(|c| &c.keys)
            .max_by(|a, b| a.value.total_cmp(&b.value))
            .unwrap()
            .time;
        for root in roots {
            enable_character_face(world, root).unwrap();
            let mut face = world.get_mut::<FacialAnimator>(root).unwrap();
            face.play(sequence.clone(), false);
            face.time = time;
            face.paused = true;
            face.fade_seconds = 0.0;
            let mut body = world.get_mut::<Animator>(root).unwrap();
            body.paused = true;
            body.random_idle = None;
            body.current.as_mut().unwrap().time = 0.7;
        }
        world.resource_mut::<Step>().0 = 1;
        info!("gear-refresh: all original actors enabled late, frozen authored A at {time}s");
    } else if step == 1 && elapsed >= 3.0 {
        let roots = world
            .query::<(Entity, &Figure)>()
            .iter(world)
            .map(|(e, f)| (e, f.0))
            .collect::<Vec<_>>();
        for (root, index) in roots {
            if index == 0 {
                continue;
            }
            let character = world.get::<Character>(root).unwrap();
            let model = character.model_root;
            let body = world.get::<Animator>(root).unwrap();
            let (rig, bones, time) = (
                body.rig.clone(),
                body.bones.clone(),
                body.current.as_ref().unwrap().time,
            );
            replace_character_parts(
                world,
                root,
                &rig,
                &bones,
                assets(if index == 1 {
                    &[22874]
                } else {
                    &[18230, 18180]
                }),
            )
            .unwrap();
            assert_eq!(world.get::<Character>(root).unwrap().model_root, model);
            let body = world.get::<Animator>(root).unwrap();
            assert_eq!(body.bones, bones);
            assert_eq!(body.current.as_ref().unwrap().time, time);
            let character = world.get::<Character>(root).unwrap();
            info!(
                "gear-refresh: actor={root:?} model={model:?} same bones={} clock={time} parts={} heel={} hair={}",
                bones.len(),
                character.parts.len(),
                character.assets.heel,
                character
                    .assets
                    .parts
                    .iter()
                    .any(|p| p.kind == PartKind::Hair)
            );
        }
        world.resource_mut::<Step>().0 = 2;
    } else if step == 2 && elapsed >= 5.0 {
        let root = world
            .query::<(Entity, &Figure)>()
            .iter(world)
            .find(|(_, f)| f.0 == 2)
            .unwrap()
            .0;
        assert!(disable_character_face(world, root));
        world.resource_mut::<Step>().0 = 3;
        info!("gear-refresh: hood face disabled after refresh; new styled-cut originals retained");
    }
}
