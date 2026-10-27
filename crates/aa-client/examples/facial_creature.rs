//! The actual creature CDF spawn path, at three authored body-clock samples, without a connection.
#![allow(dead_code)]
#[path = "../src/script/mod.rs"]
mod script;
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
#[path = "../src/task.rs"]
mod task;
#[path = "../src/lua_value.rs"]
mod lua_value;
use lua_value::lua_int;

use aa_anim::facial::{FacialAnimator, FacialPlugin};
use aa_anim::{AnimPlugin, Animator, Cal, ClipLibrary};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::mesh::morph::{MeshMorphWeights, MorphWeights};
use bevy::prelude::*;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    WorldSelect,
    Login,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

#[derive(Component)]
struct Sample {
    label: &'static str,
    time: f32,
    started: bool,
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: authored NPC facial playback through creature CDF"),
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
        .add_systems(Update, sample_body)
        .add_systems(Last, verify)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let packet = std::env::args().any(|a| a == "--packet");
    let actor = "objects/characters/people/npc_chef01/nude/npc_chef01.cdf";
    let cdf = packet.then(|| {
        let db = aa_anim::GameDb::open().unwrap();
        let mut equipment = aa_net::model::Equipment::default();
        for (slot, template_id) in [
            (aa_net::model::slot::FACE, 14478),
            (aa_net::model::slot::HAIR, 14475),
            (aa_net::model::slot::BODY, 14479),
        ] {
            equipment.slots[slot] = Some(aa_net::model::Item { template_id, ..default() });
        }
        ingame::creature::configured_cdf(&db, actor, 631, &equipment).unwrap()
    });
    for (i, (label, time)) in [("NPC / NEUTRAL", 0.0), ("NPC / AUTHORED A", 1.7218), ("NPC / RETURN", 3.0)]
        .into_iter()
        .enumerate()
    {
        // The base CDF has authored bound body/head/hair skins. The un-suffixed CDF has empty customization slots.
        let root = ingame::creature::spawn(
            &mut commands,
            &mut cry,
            &clips,
            if packet {
                actor
            } else {
                "objects/characters/people/npc_chef01/nude/npc_chef01_base.cdf"
            },
            cdf.as_ref(),
            Transform::from_xyz((i as f32 - 1.0) * 0.52, 0.0, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
        )
        .expect("actual creature CDF spawn");
        commands.entity(root).insert(Sample { label, time, started: false });
    }
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
        Transform::from_xyz(0.0, 1.55, 3.6).looking_at(Vec3::new(0.0, 1.51, 0.0), Vec3::Y),
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
                Text::new("NPC / NEUTRAL 0.00s"),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                }
            ),
            (
                Text::new("NPC / AUTHORED A 1.72s"),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                }
            ),
            (
                Text::new("NPC / RETURN 3.00s"),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                }
            ),
        ],
    ));
}

fn sample_body(clips: Res<ClipLibrary>, mut figures: Query<(&mut Sample, &mut Animator, &mut FacialAnimator)>) {
    let cal = Cal::shared("objects/characters/people/npc_chef01/nude/npc_chef01_base.cal").unwrap();
    let talk = cal.animation("fist_ac_talk_11").unwrap();
    for (mut sample, mut body, mut face) in &mut figures {
        if sample.started {
            continue;
        }
        let bound = clips.bound(&body.rig, &talk.path, aa_anim::pose::SampleOptions::default()).unwrap();
        body.play(bound, true, 0.0);
        body.speed = 0.0;
        body.random_idle = None;
        body.current.as_mut().unwrap().time = sample.time;
        face.fade_seconds = 0.0;
        assert!(face.follow_body);
        sample.started = true;
    }
}

fn verify(
    time: Res<Time>,
    figures: Query<(Entity, &Sample, &FacialAnimator, &MorphWeights)>,
    meshes: Query<(Entity, &Mesh3d, Option<&MeshMorphWeights>)>,
    parents: Query<&ChildOf>,
    mut checked: Local<bool>,
) {
    if *checked || time.elapsed_secs() < 3.0 {
        return;
    }
    assert_eq!(figures.iter().count(), 3, "all CDF instances must attach facial playback on their body root");
    let mut face_meshes = std::collections::HashSet::new();
    for (root, sample, face, weights) in &figures {
        assert_eq!(face.targets.len(), 23);
        assert!(face.sequence.is_some(), "{} must resolve the authored CAF-to-FSQ mapping", sample.label);
        assert!((face.time - sample.time).abs() < 0.0001);
        let peak = weights.weights().iter().copied().fold(0.0f32, f32::max);
        if sample.time == 1.7218 {
            assert!(peak > 0.5);
        }
        if sample.time == 3.0 {
            assert!(weights.weights().iter().all(|w| w.abs() < 0.1));
        }
        let (mut bound, mut plain) = (0, 0);
        for (entity, mesh, reference) in &meshes {
            let mut ancestor = entity;
            while ancestor != root {
                let Ok(parent) = parents.get(ancestor) else {
                    break;
                };
                ancestor = parent.parent();
            }
            if ancestor != root {
                continue;
            }
            match reference {
                Some(MeshMorphWeights::Reference(owner)) => {
                    assert_eq!(*owner, root, "a head must bind to its own body root");
                    assert!(face_meshes.insert(mesh.id()), "instances must have independent prepared mesh handles");
                    bound += 1;
                }
                None => plain += 1,
                _ => panic!("unexpected copied morph state"),
            }
        }
        assert!(
            bound > 0 && plain >= 5,
            "head subsets must bind while body/clothing/hair and both rigid eyes remain plain"
        );
        info!(
            "facial-creature: {} face_time={} peak={} facial_subsets={} plain_subsets={}",
            sample.label, face.time, peak, bound, plain
        );
    }
    *checked = true;
}
