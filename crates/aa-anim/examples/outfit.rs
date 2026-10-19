//! Whole outfits (several equipment items at once) on one or more models, standing on a floor:
//! checks how the pieces sit together (pants into boots, sleeves into gloves, robes, a costume over
//! armour), heel heights and underwear textures.
//!
//! ```text
//! cargo run -p aa-anim --example outfit -- --outfits "23387,23388,23390;28439" [--models 10,11]
//!     --shot shots/aa-anim/outfit.png [--shot-after 10]
//!     --outfits "a,b,c;d,e"   item ids per outfit, outfits separated by `;` (`-` = nude)
//!     [--models 10,11,..]     model ids (default 10); one figure per model x outfit, left to right
//!     [--view body|legs|feet|arms|head]   camera framing (default body)
//!     [--yaw deg]             turn every figure (0 = facing the camera, 90 = right side)
//!     [--spacing m]           distance between figures (default 1.0)
//! ```

use aa_anim::{AnimPlugin, CharacterDesc, ClipLibrary, Costume, GameDb, Race, spawn_character};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::prelude::*;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-anim outfit"), aa_dev::DevPlugin, CryPlugin, AnimPlugin))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.75, 0.8, 0.9), brightness: 700.0, ..default() })
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let db = GameDb::open().expect("game database");
    let outfits: Vec<Vec<u32>> = arg("--outfits")
        .unwrap_or_else(|| "23387,23388,23390".into())
        .split(';')
        .map(|o| o.split(',').filter_map(|i| i.trim().parse().ok()).collect())
        .collect();
    let models: Vec<u32> = arg("--models").unwrap_or_else(|| "10".into()).split(',').filter_map(|m| m.trim().parse().ok()).collect();
    let spacing: f32 = arg("--spacing").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let yaw: f32 = arg("--yaw").and_then(|v| v.parse().ok()).unwrap_or(0.0);

    let figures: Vec<(u32, &Vec<u32>)> = models.iter().flat_map(|&m| outfits.iter().map(move |o| (m, o))).collect();
    let n = figures.len();
    for (k, (model, items)) in figures.iter().enumerate() {
        let Some((race, gender)) = Race::from_model_id(*model) else {
            error!("unknown model {model}");
            continue;
        };
        let mut desc = CharacterDesc::default_for(&db, race, gender).expect("default look");
        desc.costume = if items.is_empty() { Costume::None } else { Costume::Items((*items).clone()) };
        let x = (k as f32 - (n as f32 - 1.0) * 0.5) * spacing;
        let rotation = Quat::from_rotation_y(std::f32::consts::PI + yaw.to_radians());
        // the hair variant of the worn headgear, as the client picks it (costume before helmet)
        let headgear = [31, 1].into_iter().find_map(|slot| {
            items
                .iter()
                .filter(|&&i| db.item_slot_type(i) == Some(slot))
                .find_map(|&i| db.item_visuals(i, *model).ok()?.first().map(|v| v.detail))
        });
        match desc.resolve_with_headgear(&db, headgear).and_then(|a| {
            info!("outfit: model {model} items {items:?}: {} parts", a.parts.len());
            spawn_character(&mut commands, &mut cry, &clips, a, Transform::from_xyz(x, 0.0, 0.0).with_rotation(rotation))
        }) {
            Ok(_) => {}
            Err(e) => error!("model {model} items {items:?}: {e}"),
        }
    }

    // floor: the soles should stand on it
    let floor = cry.meshes.add(Cuboid::new(n as f32 * spacing + 2.0, 0.02, 3.0));
    let floor_mat = cry.materials.add(StandardMaterial { base_color: Color::srgb(0.25, 0.27, 0.3), perceptual_roughness: 1.0, reflectance: 0.0, ..default() });
    commands.spawn((Mesh3d(floor), MeshMaterial3d(floor_mat), Transform::from_xyz(0.0, -0.01, 0.0)));

    for (lux, color, from) in [
        (18000.0, Color::srgb(1.0, 0.96, 0.9), Vec3::new(2.0, 5.0, 6.0)),
        (5000.0, Color::srgb(0.6, 0.7, 1.0), Vec3::new(-5.0, 2.5, 4.0)),
        (8000.0, Color::srgb(1.0, 0.9, 0.8), Vec3::new(0.0, 3.0, -6.0)),
    ] {
        commands.spawn((
            DirectionalLight { illuminance: lux, color, shadow_maps_enabled: false, ..default() },
            Transform::from_translation(from).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
        ));
    }
    // (centre height, visible height) of the view
    let (centre, extent) = match arg("--view").as_deref() {
        Some("legs") => (0.55, 1.2),
        Some("feet") => (0.15, 0.5),
        Some("arms") => (1.1, 0.9),
        Some("head") => (1.62, 0.6),
        _ => (0.95, 2.1),
    };
    let width = (n as f32 * spacing).max(extent * 16.0 / 9.0);
    let height = extent.max(width * 9.0 / 16.0);
    let dist = 12.0;
    let fov = 2.0 * (height * 0.5 / dist).atan();
    let mut camera = commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Projection::Perspective(PerspectiveProjection { fov, ..default() }),
        Transform::from_xyz(0.0, centre + 0.25, dist).looking_at(Vec3::new(0.0, centre, 0.0), Vec3::Y),
    ));
    if let Some(env) = aa_cry::stage::environment_light(&mut cry, aa_cry::stage::DEFAULT_ENVIRONMENT, 3000.0) {
        camera.insert(env);
    }
}
