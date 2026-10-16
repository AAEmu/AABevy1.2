//! Every playable race and gender side by side in the login costume, playing the stage idle.
//!
//! ```text
//! cargo run -p aa-anim --example lineup -- --shot shots/aa-anim/lineup.png [--shot-after 12]
//!     [--preset N]       character-create face preset 0..8 (default 0)
//!     [--costume none]   nude instead of the login outfit
//!     [--no-hair 1]      leave the hair out
//!     [--only N]         only character N (0..7: nuian m/f, elf m/f, hariharan m/f, ferre m/f), at the origin
//!     [--env I | --no-env]   reflection cubemap intensity (default 1500)
//!     [--cam x,y,z --look x,y,z --fov deg]   camera override (close-ups)
//! ```

use aa_anim::{AnimPlugin, CharacterDesc, ClipLibrary, Costume, GameDb, Gender, Race, spawn_character};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-anim lineup"), aa_dev::DevPlugin, CryPlugin, AnimPlugin))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.75, 0.8, 0.9), brightness: 350.0 * light_scale(), ..default() })
        .add_systems(Startup, setup)
        .run();
}

/// Overall light level of the showroom (`--light K`, default 2): the lights are in lux against
/// Bevy's default exposure, so full values wash the characters out.
fn light_scale() -> f32 {
    arg("--light").and_then(|v| v.parse().ok()).unwrap_or(2.0)
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let preset: usize = arg("--preset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let nude = arg("--costume").is_some_and(|c| c == "none");
    let db = GameDb::open().expect("game database");

    let only: Option<usize> = arg("--only").and_then(|v| v.parse().ok());
    let mut x = -3.85;
    let mut n = 0;
    for race in Race::ALL {
        for gender in Gender::ALL {
            n += 1;
            if only.is_some_and(|o| o != n - 1) {
                continue;
            }
            let x0 = if only.is_some() { 0.0 } else { x };
            x += 1.1;
            let mut desc = CharacterDesc::from_preset(&db, race, gender, preset).expect("preset");
            if arg("--no-hair").is_some() {
                desc.hair = None;
            }
            if nude {
                desc.costume = Costume::None;
            }
            match desc.resolve(&db).and_then(|a| {
                spawn_character(&mut commands, &mut cry, &clips, a, Transform::from_xyz(x0, 0.0, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::PI)))
            }) {
                Ok(_) => {}
                Err(e) => error!("{race:?} {gender:?}: {e}"),
            }
        }
    }

    if !cry.cache.missing_textures.is_empty() {
        warn!("missing textures: {:#?}", cry.cache.missing_textures);
    }

    // showroom: dark floor disc, key + fill + rim light, camera in front
    let floor = cry.meshes.add(Cylinder::new(7.0, 0.04));
    let floor_mat = cry.materials.add(StandardMaterial {
        base_color: Color::srgb(0.1, 0.105, 0.12),
        perceptual_roughness: 1.0,
        reflectance: 0.0,
        ..default()
    });
    commands.spawn((Mesh3d(floor), MeshMaterial3d(floor_mat), Transform::from_xyz(0.0, -0.02, 0.0)));

    commands.spawn((
        DirectionalLight { illuminance: 9000.0 * light_scale(), shadow_maps_enabled: true, color: Color::srgb(1.0, 0.96, 0.9), ..default() },
        Transform::from_xyz(2.0, 5.0, 6.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 2500.0 * light_scale(), color: Color::srgb(0.6, 0.7, 1.0), ..default() },
        Transform::from_xyz(-5.0, 2.5, 4.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 4000.0 * light_scale(), color: Color::srgb(1.0, 0.9, 0.8), ..default() },
        Transform::from_xyz(0.0, 3.0, -6.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    let vec3 = |name: &str, d: Vec3| {
        arg(name)
            .and_then(|v| {
                let p: Vec<f32> = v.split(',').filter_map(|c| c.trim().parse().ok()).collect();
                (p.len() == 3).then(|| Vec3::new(p[0], p[1], p[2]))
            })
            .unwrap_or(d)
    };
    let cam = vec3("--cam", Vec3::new(0.0, 1.15, 9.6));
    let look = vec3("--look", Vec3::new(0.0, 0.95, 0.0));
    let fov: f32 = arg("--fov").and_then(|v| v.parse().ok()).unwrap_or(28.0);
    let mut camera = commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Projection::Perspective(PerspectiveProjection { fov: fov.to_radians(), ..default() }),
        Transform::from_translation(cam).looking_at(look, Vec3::Y),
    ));
    // the costume materials reference the game's own beach cubemap for reflections
    if arg("--no-env").is_none() {
        let intensity: f32 = arg("--env").and_then(|v| v.parse().ok()).unwrap_or(1500.0) * light_scale();
        if let Some(env) = aa_cry::stage::environment_light(&mut cry, aa_cry::stage::DEFAULT_ENVIRONMENT, intensity) {
            camera.insert(env);
        }
    }
}
