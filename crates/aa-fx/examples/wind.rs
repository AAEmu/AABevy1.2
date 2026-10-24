//! Original-pak Wind receiver comparison. --case external|same|outside|no-force
//! --wind on|off --out shots/aa-fx/wind/external-on --size 1280x720
//! Keeps real time for FPS diagnostics; only the game clock is fixed at 60 Hz.
//! Same-owner control composes two unmodified original node trees in one run;
//! it is a test composition, not an additional authored original effect.

use aa_fx::wind::{FxWindFields, FxWindPublished, FxWindSettings};
use aa_fx::{Effect, EffectDef, FxLibraries, FxPlugin, FxRun, FxSettings, FxStats, Library};
use bevy::camera::visibility::VisibilitySystems;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn arg(key: &str, default: &str) -> String {
    let args: Vec<_> = std::env::args().collect();
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| default.into())
}

#[derive(Resource)]
struct Trial {
    case: String,
    out: String,
    preload: Vec<Entity>,
    start: Option<f64>,
    next: usize,
    done: Option<Instant>,
    launched: Instant,
}

fn fixed_game_clock(mut game: ResMut<Time>, mut clock: Local<Time>) {
    clock.advance_by(Duration::from_secs_f64(1.0 / 60.0));
    *game = *clock;
}

fn main() {
    // Existing emitter debug gate: original parameters/textures stay untouched.
    // No explosion drawing emitters obscure the separate original smoke receiver.
    unsafe {
        std::env::set_var("AA_FX_EMITTER", "wind4,ship_canon_proj_brown_smoke,smoke4");
    }
    let library = Library::load("X2_EFFECTS").expect("original library");
    let source = library
        .get("doodad_effect.quest_small_obj_explosion")
        .unwrap()
        .clone();
    let receiver = library.get("e_effect.siegetank_fire_proj").unwrap().clone();
    let noforce = library.get("e_effect.smoke").unwrap().clone();
    let mut same = (*source).clone();
    same.root.children.push(receiver.root.clone());
    let offset = same.emitters.len();
    same.emitters
        .extend(receiver.emitters.iter().cloned().map(|mut e| {
            e.parent = e.parent.map(|p| p + offset);
            e
        }));
    let mut fixtures = Library::default();
    for (name, def) in [
        ("source", source),
        ("receiver", receiver),
        ("no_force", noforce),
        ("same", Arc::new(same)),
    ] {
        fixtures.effects.insert(
            name.into(),
            Arc::new(EffectDef {
                name: name.into(),
                ..(*def).clone()
            }),
        );
    }
    let mut libraries = FxLibraries::default();
    libraries.insert("windcapture", fixtures);
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-fx original Wind comparison"))
        .add_plugins((aa_dev::DevPlugin, FxPlugin))
        .insert_resource(libraries)
        .insert_resource(FxWindSettings {
            enabled: arg("--wind", "on") == "on",
            max_fields: 64,
        })
        .insert_resource(FxSettings {
            wind: Vec3::ZERO,
            max_lights: 0,
            max_particles: 20000,
            soft_distance: 0.6,
            max_distance: 500.,
            ..default()
        })
        .insert_resource(ClearColor(Color::srgb(0.035, 0.045, 0.065)))
        .insert_resource(Trial {
            case: arg("--case", "external"),
            out: arg("--out", "shots/aa-fx/wind/external-on"),
            preload: Vec::new(),
            start: None,
            next: 0,
            done: None,
            launched: Instant::now(),
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (fixed_game_clock, start).chain())
        .add_systems(
            PostUpdate,
            capture
                .after(FxWindPublished)
                .after(VisibilitySystems::CheckVisibility),
        )
        .run();
}

fn effect(name: &str) -> Effect {
    Effect {
        prime: false,
        no_body: true,
        ..Effect::new(format!("windcapture.{name}"))
    }
}

fn setup(
    mut commands: Commands,
    mut trial: ResMut<Trial>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        DepthPrepass,
        Transform::from_xyz(0., 3.5, 15.).looking_at(Vec3::new(0., 1., 0.), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.,
            ..default()
        },
        Transform::from_xyz(4., 8., 3.).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        brightness: 500.,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(100., 100.))),
        MeshMaterial3d(materials.add(Color::srgb(0.17, 0.19, 0.22))),
    ));
    let marker = meshes.add(Cuboid::new(0.08, 1.8, 0.08));
    let material = materials.add(Color::srgb(0.45, 0.45, 0.45));
    for x in [-5., 0., 5.] {
        commands.spawn((
            Mesh3d(marker.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(x, 0.9, -2.),
        ));
    }
    for name in ["source", "receiver", "no_force", "same"] {
        trial.preload.push(
            commands
                .spawn((
                    effect(name),
                    Transform::from_xyz(0., 0., -40.),
                    Visibility::Hidden,
                ))
                .id(),
        );
    }
}

fn start(mut commands: Commands, time: Res<Time>, mut trial: ResMut<Trial>, runs: Query<&FxRun>) {
    if trial.start.is_some() || time.elapsed_secs_f64() < 4.0 {
        return;
    }
    if !trial
        .preload
        .iter()
        .all(|&id| runs.get(id).is_ok_and(|r| r.resolved()))
        || !runs.get(trial.preload[1]).is_ok_and(|r| r.particles() > 0)
        || !runs.get(trial.preload[2]).is_ok_and(|r| r.particles() > 0)
    {
        return;
    }
    for id in trial.preload.drain(..) {
        commands.entity(id).despawn();
    }
    trial.start = Some(time.elapsed_secs_f64());
    match trial.case.as_str() {
        "same" => {
            let id = commands.spawn((effect("same"), Transform::IDENTITY)).id();
            info!(
                "wind-trial owner={id:?} bits={} native-tree-composition",
                id.to_bits()
            );
        }
        "external" | "outside" | "no-force" => {
            let receiver = commands
                .spawn((
                    effect(if trial.case == "no-force" {
                        "no_force"
                    } else {
                        "receiver"
                    }),
                    Transform::IDENTITY,
                ))
                .id();
            let source = commands
                .spawn((
                    effect("source"),
                    Transform::from_xyz(if trial.case == "outside" { 50. } else { -5. }, 0., 0.),
                ))
                .id();
            info!(
                "wind-trial receiver={receiver:?} bits={} source={source:?} bits={}",
                receiver.to_bits(),
                source.to_bits()
            );
        }
        other => panic!("unknown --case {other}"),
    }
    info!(
        "wind-trial start case={} game={:.6}",
        trial.case,
        time.elapsed_secs_f64()
    );
}

fn capture(
    mut commands: Commands,
    time: Res<Time>,
    mut trial: ResMut<Trial>,
    fields: Res<FxWindFields>,
    stats: Res<FxStats>,
    mut exit: MessageWriter<AppExit>,
    mut last_log: Local<u64>,
) {
    if trial.launched.elapsed().as_secs() > 30 {
        error!("wind-trial readiness timeout");
        exit.write(AppExit::error());
        return;
    }
    let Some(start) = trial.start else {
        return;
    };
    let age = time.elapsed_secs_f64() - start;
    let frame = (age * 60.).round() as u64;
    if frame <= 60 && frame / 6 != *last_log {
        *last_log = frame / 6;
        info!(
            "wind-trial +{age:.6} fields_empty={} delta={:?} particles={} sim_ms={:.6} upload_ms={:.6}",
            fields.is_empty(),
            fields.sample(Vec3::ZERO),
            stats.particles,
            stats.sim_ms,
            stats.upload_ms
        );
    }
    let times = [0.2, 0.8];
    if trial.next < times.len() && age + 1e-8 >= times[trial.next] {
        let path = format!("{}_{}.png", trial.out, trial.next);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
        info!("wind-capture {path} +{age:.6}");
        trial.next += 1;
        if trial.next == times.len() {
            trial.done = Some(Instant::now());
        }
    }
    if trial.done.is_some_and(|t| t.elapsed().as_secs_f32() > 1.5) {
        exit.write(AppExit::Success);
    }
}
