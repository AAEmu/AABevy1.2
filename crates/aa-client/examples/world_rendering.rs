//! Offline rendering with the client's dependency features; no network session or account.
//!
//! `cargo run --locked -p aa-client --example world_rendering -- --shot shots/resume/world-rendering.png --shot-after 30`
//! Optional `--hour H` changes the fixed Royal Palace court's time of day.
use aa_world::{AaWorldPlugin, WorldClock, cry_to_bevy};
use bevy::prelude::*;

fn main() {
    let mut world = AaWorldPlugin::new("main_world");
    world.config.load_radius = 180.0;
    world.config.unload_radius = 216.0;
    let args: Vec<_> = std::env::args().collect();
    let hour = args
        .windows(2)
        .find(|v| v[0] == "--hour")
        .and_then(|v| v[1].parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(10.0);
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: offline world rendering"),
            aa_dev::DevPlugin,
        ))
        .add_plugins((aa_fx::FxPlugin, world))
        .insert_resource(WorldClock { hour, ..default() })
        .add_systems(Startup, setup)
        .add_systems(Update, report)
        .run();
}

fn setup(mut commands: Commands) {
    let position = cry_to_bevy(Vec3::new(13831.0, 10866.0, 172.29433));
    let target = cry_to_bevy(Vec3::new(13822.0, 10878.0, 166.29433));
    commands.spawn((
        Camera3d::default(),
        aa_world::WorldFocus,
        Projection::Perspective(PerspectiveProjection {
            fov: 60_f32.to_radians(),
            near: 0.3,
            far: 20000.0,
            ..default()
        }),
        Transform::from_translation(position).looking_at(target, Vec3::Y),
    ));
}

fn report(
    time: Res<Time>,
    mut last: Local<f32>,
    terrain: Query<&ViewVisibility, With<aa_world::stream::TerrainPatch>>,
    lights: Query<&aa_cry::shade::illum::IllumLightSpecular>,
    objects: Res<aa_world::objects::ObjectState>,
) {
    if time.elapsed_secs() < *last + 5.0 {
        return;
    }
    *last = time.elapsed_secs();
    info!(
        "client render fixture: terrain prepared={} visible={} object_parts={} authored_light_ratios={}",
        terrain.iter().len(),
        terrain.iter().filter(|v| v.get()).count(),
        objects.stats.entities,
        lights.iter().len()
    );
}
