//! Character-create face presets (and manual customisation) side by side, head close-up.
//!
//! ```text
//! cargo run -p aa-anim --example faces -- --shot shots/aa-anim/faces_nuian_f.png
//!     [--race nuian|elf|hariharan|ferre] [--gender m|f]
//!     [--presets 0,1,2,3,4]      create-screen presets (0..8)
//!     [--morph 4=30,12=-50;25=-60]   extra face sliders (`Idx=value`); `;` separates one list per face
//!     [--skin ID] [--hair ITEM] [--hair-color ID]   override preset values for every face
//!     [--fov DEG]                vertical field of view (zoom)
//!     [--full 1]                 full body instead of the head close-up
//! ```

use aa_anim::{AnimPlugin, CharacterDesc, ClipLibrary, GameDb, Gender, Race, spawn_character};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::light::GeneratedEnvironmentMapLight;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-anim faces"), aa_dev::DevPlugin, CryPlugin, AnimPlugin))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.75, 0.8, 0.9), brightness: 300.0 * light_scale(), ..default() })
        .add_systems(Startup, setup)
        .run();
}

/// Overall light level of the showroom (`--light K`, default 0.5): the lights are in lux against
/// Bevy's default exposure, so full values wash the characters out.
fn light_scale() -> f32 {
    arg("--light").and_then(|v| v.parse().ok()).unwrap_or(0.5)
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let race = match arg("--race").as_deref() {
        Some("elf") => Race::Elf,
        Some("hariharan") | Some("harani") => Race::Hariharan,
        Some("ferre") | Some("firran") => Race::Ferre,
        _ => Race::Nuian,
    };
    let gender = if arg("--gender").is_some_and(|g| g.starts_with('f')) { Gender::Female } else { Gender::Male };
    let presets: Vec<usize> = arg("--presets")
        .map(|v| v.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![0, 1, 2, 3, 4]);
    // `--morph "4=35,25=65;25=-80"`: one `;`-separated list of `Idx=value` per face (a single list applies to all)
    let extra: Vec<Vec<(usize, i8)>> = arg("--morph")
        .map(|v| {
            v.split(';')
                .map(|list| {
                    list.split(',')
                        .filter_map(|kv| {
                            let (k, v) = kv.split_once('=')?;
                            Some((k.trim().parse().ok()?, v.trim().parse().ok()?))
                        })
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();
    let full = arg("--full").is_some();
    let db = GameDb::open().expect("game database");

    let spacing = if full { 1.1 } else { 0.5 };
    let x0 = -spacing * (presets.len() as f32 - 1.0) / 2.0;
    for (i, &p) in presets.iter().enumerate() {
        let mut desc = CharacterDesc::from_preset(&db, race, gender, p).expect("preset");
        let mine = if extra.len() == 1 { extra.first() } else { extra.get(i) };
        for &(idx, v) in mine.into_iter().flatten() {
            desc.customization.set_slider(idx, v);
        }
        if let Some(v) = arg("--skin").and_then(|v| v.parse().ok()) {
            desc.skin = v;
        }
        if let Some(v) = arg("--hair").and_then(|v| v.parse().ok()) {
            desc.hair = Some(v);
        }
        if let Some(v) = arg("--hair-color").and_then(|v| v.parse().ok()) {
            desc.hair_color = Some(v);
        }
        let at = Transform::from_xyz(x0 + i as f32 * spacing, 0.0, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::PI));
        if let Err(e) = desc.resolve(&db).and_then(|a| spawn_character(&mut commands, &mut cry, &clips, a, at)) {
            error!("preset {p}: {e}");
        }
    }

    commands.spawn((
        DirectionalLight { illuminance: 9000.0 * light_scale(), shadow_maps_enabled: true, color: Color::srgb(1.0, 0.96, 0.9), ..default() },
        Transform::from_xyz(1.5, 3.0, 5.0).looking_at(Vec3::new(0.0, 1.5, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 2500.0 * light_scale(), color: Color::srgb(0.6, 0.7, 1.0), ..default() },
        Transform::from_xyz(-4.0, 2.0, 3.0).looking_at(Vec3::new(0.0, 1.5, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 3000.0 * light_scale(), color: Color::srgb(1.0, 0.9, 0.8), ..default() },
        Transform::from_xyz(0.0, 3.0, -5.0).looking_at(Vec3::new(0.0, 1.5, 0.0), Vec3::Y),
    ));

    let (cam, look, fov): (Vec3, Vec3, f32) = if full {
        (Vec3::new(0.0, 1.1, 11.0), Vec3::new(0.0, 0.95, 0.0), 22.0)
    } else {
        // face height of each model (metres), for framing
        let head = match (race, gender) {
            (Race::Nuian, Gender::Male) => 1.72,
            (Race::Nuian, _) => 1.62,
            (Race::Elf, Gender::Male) => 1.72,
            (Race::Elf, _) => 1.62,
            (Race::Hariharan, Gender::Male) => 1.58,
            (Race::Hariharan, _) => 1.47,
            (Race::Ferre, Gender::Male) => 1.9,
            (Race::Ferre, _) => 1.7,
        };
        (Vec3::new(0.0, head, 3.6), Vec3::new(0.0, head - 0.05, 0.0), 22.0)
    };
    let fov = arg("--fov").and_then(|v| v.parse().ok()).unwrap_or(fov);
    let mut camera = commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: fov.to_radians(), ..default() }),
        Transform::from_translation(cam).looking_at(look, Vec3::Y),
    ));
    if let Some(env) = cry.texture("textures/cubemaps/beach_blur2.dds", true) {
        camera.insert(GeneratedEnvironmentMapLight { environment_map: env, intensity: 1500.0 * light_scale(), ..default() });
    }
}
