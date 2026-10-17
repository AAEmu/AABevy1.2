//! Character select with a 3D scene between the painted background and the UI panels (split
//! mode), and a stage switch from world select (`ReloadUi`).
//!
//! ```text
//! cargo run -p aa-ui --example character_select_3d -- --shot shots/aa-ui/character_select_3d.png
//! cargo run -p aa-ui --example character_select_3d -- --from-world-select 3 --shot-after 6 --shot shots/aa-ui/stage_switch.png
//! ```
//!
//! The `background` UI layer (the login backgrounds, `SetUILayer("background")`) is drawn by a
//! camera at order -10, the 3D camera (a spinning cube) at order 0, the rest of the UI at 10.
//! `--no-ui` shows the 3D layer alone. `--from-world-select S` starts on world select and switches to character select after S
//! seconds with `commands.queue(ReloadUi::new(..))`, a fresh Lua state like the client's stage
//! change.

#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn character_select(ui: &mut aa_ui::UiRuntime) {
    let worlds = fake::worlds_from_args();
    let chars = fake::characters_from_args();
    fake::install_with(ui, "en_us", &worlds, &chars);
    ui.run_stage(aa_ui::Stage::CharacterSelect);
    ui.fire_event("SHOW_CHARACTER_SELECT_WINDOW", true);
    fake::report(ui, "character_select");
}

fn world_select(ui: &mut aa_ui::UiRuntime) {
    let worlds = fake::worlds_from_args();
    fake::install_with(ui, "en_us", &worlds, &[]);
    ui.run_stage(aa_ui::Stage::WorldSelect);
    ui.fire_event("ENTERED_WORLD_SELECT", ());
    ui.fire_event("SHOW_SERVER_SELECT_WINDOW", true);
    fake::report(ui, "world_select");
}

fn main() {
    let switch_at: Option<f32> = arg("--from-world-select").and_then(|s| s.parse().ok());
    let config = aa_ui::UiConfig {
        background_camera_order: Some(-10),
        camera_order: 10,
        ..fake::config()
    };
    let mut app = App::new();
    app.add_plugins(aa_dev::default_plugins("aa-ui: character select 3d"))
        .add_plugins(aa_dev::DevPlugin);
    app.add_systems(Startup, (fake::physical_pixels, scene))
        .add_systems(Update, spin);
    // `--no-ui`: the 3D layer alone.
    if !std::env::args().any(|a| a == "--no-ui") {
        app.add_plugins(aa_ui::UiPlugin {
            config: Some(config),
        })
        .insert_resource(match switch_at {
            Some(_) => aa_ui::UiSetup::new(world_select),
            None => aa_ui::UiSetup::new(character_select),
        })
        .insert_resource(SwitchAt(switch_at))
        .add_systems(
            Update,
            (switch_stage, fake::dump_tree, fake::scripted_input),
        );
    }
    app.run();
}

#[derive(Resource)]
struct SwitchAt(Option<f32>);

#[derive(Component)]
struct Spin;

/// The 3D layer: a lit cube where the character model stands, seen by a camera that draws over
/// the background camera's output (no colour clear) and under the UI camera.
fn scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 0,
            // Draw over the background camera's output.
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Transform::from_xyz(0.0, 1.2, 5.0).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.0,
            ..default()
        },
        Transform::from_xyz(3.0, 6.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.9, 0.9, 0.9))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.8, 0.25, 0.2),
            ..default()
        })),
        Transform::from_xyz(-1.9, 0.9, 0.0),
        Spin,
    ));
}

fn spin(time: Res<Time>, mut q: Query<&mut Transform, With<Spin>>) {
    for mut t in &mut q {
        t.rotation = Quat::from_euler(EulerRot::YXZ, time.elapsed_secs() * 0.8, 0.4, 0.0);
    }
}

/// `--from-world-select S`: the stage change.
fn switch_stage(mut commands: Commands, time: Res<Time>, mut at: ResMut<SwitchAt>) {
    if at.0.is_some_and(|s| time.elapsed_secs() >= s) {
        at.0 = None;
        info!("character_select_3d: stage change -> character select");
        commands.queue(aa_ui::ReloadUi::new(character_select));
    }
}
