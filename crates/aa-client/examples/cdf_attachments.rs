//! Actual creature and resolved character-part CDF effect bindings, without a connection.
#![allow(dead_code)]
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
use aa_anim::{AnimPlugin, CharacterDesc, ClipLibrary, Costume, GameDb, Gender, Race};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::prelude::*;
#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}
#[derive(Component)]
struct Actor;
#[derive(Resource, Default)]
struct Phase(u8);

fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("aa-client: original CDF bone effects"),
        aa_dev::DevPlugin,
        CryPlugin,
        AnimPlugin,
        aa_fx::FxPlugin,
    ))
    .add_plugins(ingame::attachments::plugin)
    .insert_resource(ClearColor(Color::srgb(0.035, 0.04, 0.055)))
    .insert_resource(GlobalAmbientLight {
        brightness: 600.0,
        ..default()
    })
    .init_resource::<Phase>()
    .add_systems(Startup, setup)
    .add_systems(Update, visibility_cycle)
    .add_systems(Last, verify);
    app.run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let horse = ingame::creature::spawn(
        &mut commands,
        &mut cry,
        &clips,
        "objects/characters/animals/horse/horse_lord.cdf",
        None,
        Transform::from_xyz(-1.8, 0.0, 0.0).with_rotation(Quat::from_rotation_y(0.55)),
    )
    .unwrap();
    commands.entity(horse).insert(Actor);
    let db = GameDb::open().unwrap();
    let mut desc = CharacterDesc::default_for(&db, Race::Elf, Gender::Female).unwrap();
    desc.costume = Costume::Meshes(vec!["objects/characters/elf/female/costume/el_f_sk_whiteday001/el_f_sk_whiteday001.cdf".into()]);
    let character = aa_anim::spawn_character(
        &mut commands,
        &mut cry,
        &clips,
        desc.resolve(&db).unwrap(),
        Transform::from_xyz(1.7, 0.0, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
    )
    .unwrap();
    commands.entity(character).insert(Actor);
    commands.spawn((
        Mesh3d(cry.meshes.add(Plane3d::default().mesh().size(16.0, 16.0))),
        MeshMaterial3d(cry.materials.add(StandardMaterial {
            base_color: Color::srgb(0.12, 0.13, 0.15),
            perceptual_roughness: 1.0,
            ..default()
        })),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 18000.0,
            ..default()
        },
        Transform::from_xyz(2.0, 5.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Transform::from_xyz(5.5, 3.4, 8.0).looking_at(Vec3::new(0.0, 1.2, 0.0), Vec3::Y),
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(20),
            top: px(20),
            ..default()
        },
        Text::new("Original horse hoof effects + resolved white-day head ring\nActors hide at 3 s, show at 4 s; --unload removes both at 7 s"),
        TextFont {
            font_size: FontSize::Px(20.0),
            ..default()
        },
    ));
}

fn visibility_cycle(time: Res<Time>, mut phase: ResMut<Phase>, mut commands: Commands, mut actors: Query<(Entity, &mut Visibility), With<Actor>>) {
    let t = time.elapsed_secs();
    if phase.0 == 0 && t >= 3.0 {
        for (_, mut visibility) in &mut actors {
            *visibility = Visibility::Hidden;
        }
        phase.0 = 1;
    } else if phase.0 == 1 && t >= 4.0 {
        for (_, mut visibility) in &mut actors {
            *visibility = Visibility::Inherited;
        }
        phase.0 = 2;
    } else if phase.0 == 2 && t >= 7.0 && std::env::args().any(|a| a == "--unload") {
        for (entity, _) in &actors {
            commands.entity(entity).despawn();
        }
        phase.0 = 3;
    }
}

fn verify(
    time: Res<Time>,
    phase: Res<Phase>,
    effects: Query<(&aa_fx::Effect, &aa_fx::FxRun)>,
    meshes: Query<(), With<aa_fx::FxMesh>>,
    mut reported: Local<u8>,
) {
    if time.elapsed_secs() < 6.0 {
        return;
    }
    if phase.0 == 3 && time.elapsed_secs() > 7.5 {
        assert_eq!(effects.iter().count(), 0);
        assert_eq!(meshes.iter().count(), 0);
        if *reported != 2 {
            info!("CDF effects: actor unload has zero effect/particle mesh entities");
            *reported = 2;
        }
    } else if phase.0 == 2 {
        assert_eq!(effects.iter().count(), 5, "four original hoof effects plus one retained part-CDF head ring");
        assert!(effects.iter().all(|(_, run)| run.resolved() && !run.is_missing()));
        if *reported == 0 {
            info!(
                "CDF effects: 5 original bindings resolved after hide/show; {} particles, {} FX meshes",
                effects.iter().map(|(_, r)| r.particles()).sum::<usize>(),
                meshes.iter().count()
            );
            *reported = 1;
        }
    }
}
