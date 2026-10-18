//! Client stage/local-player spawn paths at two authored body-clock samples, without a connection.
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

use aa_anim::facial::{FacialAnimator, FacialPlugin, FacialSequence};
use aa_anim::{AnimPlugin, Animator, CharacterDesc, ClipLibrary, GameDb, Gender, Race};
use aa_cry::scene::{CryAssets, CryPlugin};
use aa_cry::stage::StageLighting;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::morph::MorphWeights;
use bevy::prelude::*;
use login::stage3d::{self, StagePose, StageSlot, StageSubject};

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
    blink: bool,
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: facial stage and local-player integration"),
            aa_dev::DevPlugin,
            CryPlugin,
            AnimPlugin,
            FacialPlugin,
            stage3d::Stage3dPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.08, 0.09, 0.11)))
        .add_systems(Startup, setup)
        .add_systems(Update, sample_body)
        .add_systems(Last, verify)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let db = GameDb::open().expect("read-only game database");
    let idle = FacialSequence::load("animations/nuian/male/facial/fist_ac_stage_idle.fsq").unwrap();
    let blink = idle
        .channels
        .iter()
        .filter(|c| c.name.eq_ignore_ascii_case("L_close_suprise"))
        .flat_map(|c| &c.splines)
        .flat_map(|s| &s.keys)
        .max_by(|a, b| a.value.total_cmp(&b.value))
        .unwrap()
        .time;
    let pose = StagePose {
        pos: stage3d::SELECT_FEET + Vec3::new(0.0, 1.5, 1.72),
        dir: Vec3::new(0.0, -1.0, -0.04).normalize(),
        roll_deg: 0.0,
        fov_deg: 32.0,
    };
    let stage = stage3d::spawn_stage(&mut commands, &mut cry, &pose, &StageLighting::login());
    for (i, (label, time)) in [("stage / open", 0.7), ("local player / blink", blink)].into_iter().enumerate() {
        let mut desc = CharacterDesc::from_preset(&db, Race::Nuian, Gender::Male, 0).unwrap();
        desc.hair = None;
        let assets = desc.resolve(&db).unwrap();
        let slot = StageSlot::facing(stage3d::SELECT_FEET + Vec3::X * ((i as f32 - 0.5) * 0.6), pose.pos, Race::Nuian, Gender::Male);
        let root = if i == 0 {
            stage3d::spawn_subject(&mut commands, &mut cry, &clips, assets, &slot, stage.root).unwrap()
        } else {
            let root = ingame::player::spawn_with_locomotion(&mut commands, &mut cry, &clips, assets, slot.transform(), vec![]).unwrap();
            // Only the harness places the world-player hierarchy on the login-stage layer.
            commands
                .entity(root)
                .insert((StageSubject, RenderLayers::layer(stage3d::STAGE_LAYER), ChildOf(stage.root)));
            root
        };
        commands.entity(root).insert(Sample { label, time, blink: i == 1 });
    }
}

fn sample_body(mut figures: Query<(&Sample, &mut Animator, &mut FacialAnimator)>) {
    for (sample, mut body, mut face) in &mut figures {
        body.speed = 0.0;
        body.random_idle = None;
        body.current.as_mut().expect("stage idle CAF").time = sample.time;
        face.fade_seconds = 0.0;
        assert!(face.follow_body, "{} must use the body's authored FSQ mapping", sample.label);
    }
}

fn verify(time: Res<Time>, figures: Query<(&Sample, &FacialAnimator, &MorphWeights)>, mut checked: Local<bool>) {
    if *checked || time.elapsed_secs() < 3.0 {
        return;
    }
    assert_eq!(figures.iter().count(), 2, "both actual client spawn paths must attach facial playback");
    for (sample, face, weights) in &figures {
        assert!(face.sequence.is_some(), "{} must resolve its CAF to an authored FSQ", sample.label);
        assert!((face.time - sample.time).abs() < 0.0001, "{} must stay on the body clock", sample.label);
        let peak = weights.weights().iter().copied().fold(0.0f32, f32::max);
        if sample.blink {
            assert!(peak > 0.5, "the authored blink must reach GPU morph weights");
        }
        info!(
            "facial-client: {} body_time={} face_time={} peak={}",
            sample.label, sample.time, face.time, peak
        );
    }
    *checked = true;
}
