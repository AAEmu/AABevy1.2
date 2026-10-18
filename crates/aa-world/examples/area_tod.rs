//! Real login2 Group-14 polygon and original cinema camera. --outside and
//! --no-subject are controls. --motion-check traverses the authored 3 m border;
//! --unload-check exercises environment off/world switch with bounded jobs.
use aa_world::{AaWorldPlugin, AreaToDState, AreaToDSubject, WorldClock, WorldConfig, cry_to_bevy};
use bevy::prelude::*;

#[derive(Component)]
struct Subject;

fn flag(name: &str) -> bool {
    std::env::args().any(|arg| arg == name)
}

fn main() {
    let mut config = WorldConfig::new("login2");
    config.load_radius = 160.0;
    config.unload_radius = 300.0;
    config.brush_distance = 500.0;
    config.vegetation = false;
    config.objects = !flag("--no-objects");
    config.shadows = false;
    let pose = aa_world::cinema::CinemaCamera::load("login2", "03_character_select_s")
        .unwrap()
        .pose(0.0);
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world native polygon area ToD"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock {
            hour: 13.0,
            ..default()
        })
        .add_systems(Startup, move |mut commands: Commands| {
            commands.spawn((
                Camera3d::default(),
                Transform::from_translation(cry_to_bevy(pose.position))
                    .looking_to(cry_to_bevy(pose.forward()), Vec3::Y),
                Projection::Perspective(PerspectiveProjection {
                    fov: pose.fov_deg.to_radians(),
                    far: 20000.0,
                    ..default()
                }),
            ));
            let mut subject = commands.spawn((
                Subject,
                Transform::from_translation(cry_to_bevy(
                    if flag("--outside") || flag("--motion-check") {
                        Vec3::new(1618.0, 429.0, 150.0)
                    } else {
                        Vec3::new(1652.0, 465.0, 150.0)
                    },
                )),
            ));
            if !flag("--no-subject") {
                subject.insert(AreaToDSubject);
            }
        })
        .add_systems(Update, advance.before(aa_world::sky::update_environment))
        .add_systems(PostUpdate, verify)
        .run();
}

fn advance(
    time: Res<Time>,
    mut subjects: Query<(&mut Transform, &mut GlobalTransform), With<Subject>>,
    mut cameras: Query<(&mut Transform, &mut GlobalTransform), (With<Camera3d>, Without<Subject>)>,
    mut config: ResMut<WorldConfig>,
) {
    if flag("--motion-check") {
        let progress = ((time.elapsed_secs() - 3.0) / 8.0).clamp(0.0, 1.0);
        for (mut transform, mut global) in &mut subjects {
            transform.translation = cry_to_bevy(Vec3::new(
                1618.0 + progress * 8.0,
                429.0 + progress * 10.0,
                150.0,
            ));
            *global = GlobalTransform::from(*transform);
        }
    }
    if flag("--unload-check") {
        if time.elapsed_secs() > 10.0 {
            config.environment = false;
        }
        if time.elapsed_secs() > 12.0 {
            config.world = "main_world".into();
        }
    }
    if flag("--cell-unload-check") && time.elapsed_secs() > 10.0 {
        for (mut transform, mut global) in &mut cameras {
            transform.translation = cry_to_bevy(Vec3::splat(-100000.0));
            *global = GlobalTransform::from(*transform);
        }
    }
}

fn verify(
    time: Res<Time>,
    state: Res<AreaToDState>,
    mut report: Local<f32>,
    mut partial: Local<bool>,
    mut done: Local<bool>,
) {
    assert!(state.pending_cells() <= 2 && state.active_inputs() <= 10);
    if state.total_weight() > 0.0 && state.total_weight() < 1.0 {
        *partial = true;
    }
    if time.elapsed_secs() - *report > 2.0 {
        *report = time.elapsed_secs();
        info!(
            "area ToD: cells={} areas={} pending={} inputs={} weight={:.6}",
            state.loaded_cells(),
            state.loaded_areas(),
            state.pending_cells(),
            state.active_inputs(),
            state.total_weight()
        );
    }
    if time.elapsed_secs() < 13.0 || *done {
        return;
    }
    *done = true;
    if flag("--unload-check") || flag("--cell-unload-check") {
        assert_eq!(
            (
                state.loaded_cells(),
                state.loaded_areas(),
                state.active_inputs(),
                state.total_weight()
            ),
            (0, 0, 0, 0.0)
        );
        info!("area ToD: unload cleared resident curves and native registrations");
    } else if flag("--outside") || flag("--no-subject") {
        assert_eq!(state.active_inputs(), 0);
    } else {
        assert_eq!(state.active_inputs(), 1);
        assert_eq!(state.total_weight(), 1.0);
        if flag("--motion-check") {
            assert!(
                *partial,
                "authored border must produce intermediate weights"
            );
        }
        info!("area ToD: native authored area + border checks passed");
    }
}
