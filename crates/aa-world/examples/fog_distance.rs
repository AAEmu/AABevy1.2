//! Original cell021_008 Fog4 through the real FogVolume material/depth path.
//! --legacy-control uses explicit initial ratio100; --fov-control shares eye distance.
//! A screenshot is required: its actual readback asserts both native per-view decisions.
use aa_world::fog::{self, FogMaterial, FogVolume, FogVolumePlugin};
use bevy::{
    asset::AssetId,
    camera::{Exposure, Viewport},
    core_pipeline::{
        prepass::DepthPrepass,
        tonemapping::{DebandDither, Tonemapping},
    },
    prelude::*,
    render::{
        batching::NoAutomaticBatching,
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const BACK: Vec3 = Vec3::new(0.12, 0.24, 0.4);
const CELL: (i32, i32) = (21, 8);
// Actual original-x86 getter300B6900 for Fog4's final world AABB, general60,
// engine factor1 and native first-scene byte10. Not a centre-distance limit.
const NATIVE_LIMIT: f32 = 281.424_93;

fn flag(v: &str) -> bool {
    std::env::args().any(|a| a == v)
}

#[derive(Resource)]
struct Control {
    volume: FogVolume,
    eyes: [Vec3; 2],
    fovs: [f32; 2],
    bounds: [Vec3; 2],
    expected: [bool; 2],
    node: Entity,
    mesh: AssetId<Mesh>,
    material: AssetId<FogMaterial>,
}
#[derive(Resource, Default)]
struct Cycle {
    stage: u8,
    started: Option<f32>,
    requested_at: Option<f32>,
    validated: u8,
    readbacks: u8,
    failed: bool,
    finished: bool,
}
impl Cycle {
    // Acknowledgements come only after the real pixels observer validates both
    // views. CPU lifecycle tests exercise this protocol, not fabricated pixels.
    fn acknowledge(&mut self, stage: u8) -> bool {
        if self.failed
            || self.finished
            || stage != self.stage
            || self.requested_at.is_none()
            || self.validated & (1 << stage) != 0
        {
            return false;
        }
        self.validated |= 1 << stage;
        true
    }
}
#[derive(Resource, Clone)]
struct Capture {
    output: PathBuf,
    after: f32,
    cycle: bool,
    offscreen: bool,
    completion: Arc<AtomicBool>,
}
impl Capture {
    fn required(&self) -> u8 {
        if self.cycle { 31 } else { 1 }
    }
    fn settle(&self) -> f32 {
        if self.cycle {
            self.after / 5.0
        } else {
            self.after
        }
    }
}
#[derive(Component)]
struct RequestStage(u8);
#[derive(Component)]
struct Label;

fn owned_dev_args(mut args: aa_dev::DevArgs) -> aa_dev::DevArgs {
    // DevPlugin has already created/installed any OffscreenTarget and camera
    // retargeting. Disable only its autonomous capture/exit paths in THIS app.
    args.shot = None;
    args.offscreen_shot = None;
    args.exit_after = None;
    args.record = None;
    args
}
fn process_result(exit: AppExit, complete: bool) -> AppExit {
    if complete { exit } else { AppExit::error() }
}
fn main() -> AppExit {
    let args = aa_dev::DevArgs::from_env();
    assert!(args.shot_after.is_finite() && args.shot_after >= 0.0);
    assert!(
        args.shot.is_some() ^ args.offscreen_shot.is_some(),
        "choose exactly one --shot or --offscreen-shot: real readback assertions are required"
    );
    assert!(
        args.exit_after.is_none() && args.record.is_none(),
        "early exit/record modes would bypass required readback assertions"
    );
    assert!(
        !flag("--cycle") || args.shot_after >= 18.0,
        "--cycle requires --shot-after18 or later for all five validated stages"
    );
    let completion = Arc::new(AtomicBool::new(false));
    let capture = Capture {
        output: PathBuf::from(args.shot.as_ref().or(args.offscreen_shot.as_ref()).unwrap()),
        after: args.shot_after,
        cycle: flag("--cycle"),
        offscreen: args.offscreen_shot.is_some(),
        completion: completion.clone(),
    };
    let exit = App::new()
        .add_plugins((
            aa_dev::default_plugins("Native per-view authored fog distance"),
            aa_dev::DevPlugin,
            FogVolumePlugin,
        ))
        .insert_resource(owned_dev_args(args))
        .insert_resource(capture)
        .insert_resource(aa_world::WorldConfig::new("main_world"))
        .insert_resource(ClearColor(Color::linear_rgb(BACK.x, BACK.y, BACK.z)))
        .init_resource::<Cycle>()
        .add_systems(Startup, setup)
        .add_systems(Update, (drive_capture, apply_cycle).chain())
        .add_observer(pixels)
        .run();
    // This return is the process's Termination value. Even an external window
    // close/AppExit::Success cannot report success before the validation receipt.
    process_result(exit, completion.load(Ordering::Acquire))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        asset::RenderAssetUsages,
        camera::RenderTarget,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
        window::WindowRef,
    };
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    // Real controller system/commands/messages, with explicit validation ACKs.
    // No renderer is installed and no GPU pixel validity is claimed here.
    fn lifecycle(cycle: bool, offscreen: bool, output: PathBuf) -> App {
        let mut app = App::new();
        app.add_message::<AppExit>()
            .init_resource::<Time>()
            .init_resource::<Cycle>()
            .insert_resource(Capture {
                output,
                after: if cycle { 5.0 } else { 1.0 },
                cycle,
                offscreen,
                completion: Arc::new(AtomicBool::new(false)),
            })
            .add_systems(Update, drive_capture);
        app
    }
    fn tick(app: &mut App, seconds: f32) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(seconds));
        app.world_mut().run_schedule(Update);
    }
    fn exits(app: &App) -> Vec<AppExit> {
        app.world()
            .resource::<Messages<AppExit>>()
            .iter_current_update_messages()
            .cloned()
            .collect()
    }
    fn request(app: &mut App) -> (Entity, u8) {
        let mut query = app.world_mut().query::<(Entity, &RequestStage)>();
        let (entity, stage) = query.single(app.world()).unwrap();
        (entity, stage.0)
    }

    #[test]
    fn every_cycle_stage_uses_actual_offscreen_target_and_requires_all_acknowledgements() {
        let image = Handle::<Image>::default();
        let mut app = lifecycle(true, true, PathBuf::from("unused-cpu-control.png"));
        app.insert_resource(aa_dev::OffscreenTarget(image.clone()));
        tick(&mut app, 0.0);
        for stage in 0..5 {
            tick(&mut app, 1.0);
            let (entity, requested) = request(&mut app);
            assert_eq!(requested, stage);
            let target = &app.world().get::<Screenshot>(entity).unwrap().0;
            assert!(matches!(target,RenderTarget::Image(t) if t.handle.id()==image.id()));
            tick(&mut app, 2.0); // delayed ACK must hold this actual stage
            assert_eq!(app.world().resource::<Cycle>().stage, stage);
            assert!(exits(&app).is_empty());
            assert!(
                !app.world()
                    .resource::<Capture>()
                    .completion
                    .load(Ordering::Acquire)
            );
            assert!(
                !app.world_mut()
                    .resource_mut::<Cycle>()
                    .acknowledge(stage + 1)
            );
            assert!(app.world_mut().resource_mut::<Cycle>().acknowledge(stage));
            app.world_mut().despawn(entity); // explicit capture-service cleanup, no fake image
            tick(&mut app, 0.0);
            if stage < 4 {
                assert!(exits(&app).is_empty());
            }
        }
        assert_eq!(app.world().resource::<Cycle>().validated, 31);
        assert!(
            app.world()
                .resource::<Capture>()
                .completion
                .load(Ordering::Acquire)
        );
        assert!(matches!(exits(&app).as_slice(), [AppExit::Success]));
        assert!(process_result(AppExit::Success, true).is_success());
    }

    #[test]
    fn window_target_is_preserved_and_missing_offscreen_target_fails_without_fallback() {
        let mut window = lifecycle(false, false, PathBuf::from("unused-window-control.png"));
        tick(&mut window, 0.0);
        tick(&mut window, 1.0);
        let (entity, stage) = request(&mut window);
        assert_eq!(stage, 0);
        assert!(matches!(
            window.world().get::<Screenshot>(entity).unwrap().0,
            RenderTarget::Window(WindowRef::Primary)
        ));
        let mut missing = lifecycle(false, true, PathBuf::from("unused-image-control.png"));
        tick(&mut missing, 0.0);
        tick(&mut missing, 1.0);
        assert!(missing.world().resource::<Cycle>().failed);
        assert!(matches!(exits(&missing).as_slice(), [AppExit::Error(_)]));
        assert!(
            !missing
                .world()
                .resource::<Capture>()
                .completion
                .load(Ordering::Acquire)
        );
        assert_eq!(
            missing
                .world_mut()
                .query::<&Screenshot>()
                .iter(missing.world())
                .count(),
            0
        );
    }

    #[test]
    fn stale_png_missing_or_delayed_capture_cannot_exit_successfully() {
        let path = std::env::temp_dir().join(format!(
            "aa-world-fog-stale-{}-{}.png",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Image::new_fill(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[97, 134, 170, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD,
        )
        .try_into_dynamic()
        .unwrap()
        .save(&path)
        .unwrap();
        let mut args = aa_dev::DevArgs::from_env();
        args.shot = Some(path.to_string_lossy().into());
        args.exit_after = Some(1.0);
        let owned = owned_dev_args(args);
        assert!(
            owned.shot.is_none() && owned.offscreen_shot.is_none() && owned.exit_after.is_none()
        );
        let mut missing = lifecycle(false, false, path.clone());
        tick(&mut missing, 0.0);
        tick(&mut missing, 1.0);
        tick(&mut missing, 2.0);
        assert!(path.exists()); // old DevPlugin's existence+1.5s shortcut would succeed
        assert!(exits(&missing).is_empty());
        assert!(process_result(AppExit::Success, false).is_error());
        tick(&mut missing, 20.0);
        assert!(missing.world().resource::<Cycle>().failed);
        assert!(matches!(exits(&missing).as_slice(), [AppExit::Error(_)]));
        assert!(!missing.world_mut().resource_mut::<Cycle>().acknowledge(0));
        let mut delayed = lifecycle(false, false, path.clone());
        tick(&mut delayed, 0.0);
        tick(&mut delayed, 1.0);
        tick(&mut delayed, 2.0);
        assert!(exits(&delayed).is_empty());
        assert!(delayed.world_mut().resource_mut::<Cycle>().acknowledge(0));
        tick(&mut delayed, 0.0);
        assert!(matches!(exits(&delayed).as_slice(), [AppExit::Success]));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn incomplete_mask_and_early_real_app_exit_are_process_failures() {
        let mut incomplete = lifecycle(true, false, PathBuf::from("unused-incomplete-control.png"));
        {
            let mut state = incomplete.world_mut().resource_mut::<Cycle>();
            state.stage = 4;
            state.validated = 16;
        }
        tick(&mut incomplete, 0.0);
        assert!(matches!(exits(&incomplete).as_slice(), [AppExit::Error(_)]));
        assert!(
            !incomplete
                .world()
                .resource::<Capture>()
                .completion
                .load(Ordering::Acquire)
        );
        let mut app = App::new();
        app.add_systems(Update, |mut exit: MessageWriter<AppExit>| {
            exit.write(AppExit::Success);
        });
        let exit = app.run(); // actual CPU runner result, no renderer/pixels implied
        assert!(exit.is_success());
        assert!(process_result(exit, false).is_error());
        assert!(process_result(AppExit::error(), true).is_error());
    }
}

fn capture_target(offscreen: bool, target: Option<&aa_dev::OffscreenTarget>) -> Option<Screenshot> {
    if offscreen {
        target.map(|t| Screenshot::image(t.0.clone()))
    } else {
        Some(Screenshot::primary_window())
    }
}

fn drive_capture(
    mut commands: Commands,
    time: Res<Time>,
    capture: Res<Capture>,
    target: Option<Res<aa_dev::OffscreenTarget>>,
    mut state: ResMut<Cycle>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed || state.finished {
        return;
    }
    let now = time.elapsed_secs();
    let started = *state.started.get_or_insert(now);
    if state.validated == capture.required() {
        state.finished = true;
        capture.completion.store(true, Ordering::Release);
        exit.write(AppExit::Success);
        return;
    }
    if state.validated & (1 << state.stage) != 0 {
        if state.stage >= 4 || !capture.cycle {
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
        state.stage += 1;
        state.started = Some(now);
        state.requested_at = None;
        return; // new config settles before that stage's image request
    }
    if now - started > capture.settle() + 15.0 {
        state.failed = true;
        error!(
            "Fog4 stage{} timed out without validated real readback",
            state.stage
        );
        exit.write(AppExit::error());
        return;
    }
    if state.requested_at.is_none() && now - started >= capture.settle() {
        if let Some(shot) = capture_target(capture.offscreen, target.as_deref()) {
            commands.spawn((shot, RequestStage(state.stage)));
            state.requested_at = Some(now);
        } else {
            state.failed = true;
            error!("Offscreen capture target absent; no window fallback or successful exit");
            exit.write(AppExit::error());
        }
    }
}

fn apply_cycle(
    mut commands: Commands,
    capture: Res<Capture>,
    c: Res<Control>,
    state: Res<Cycle>,
    mut last: Local<Option<u8>>,
    mut config: ResMut<aa_world::WorldConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut labels: Query<&mut Text, With<Label>>,
) {
    if !capture.cycle || *last == Some(state.stage) {
        return;
    }
    *last = Some(state.stage);
    let stage = state.stage;
    config.view_dist_scale = match stage {
        1 => 0.15,
        2 => 3.0,
        _ => 1.5,
    };
    if stage == 4 {
        commands.entity(c.node).despawn();
        meshes.remove(c.mesh);
        materials.remove(c.material);
    }
    if stage != 0 {
        for mut label in &mut labels {
            label.0 = format!(
                "Original cell021_008 Fog4 / validated live stage{stage}\nNative general{}; {}\nEach view readback checks native admission; handles stay fixed until unload.",
                config.view_dist_scale * 40.0,
                match stage {
                    1 => "low distance control",
                    2 => "high distance control",
                    3 => "restored distance control",
                    _ => "unloaded node/mesh/material",
                }
            );
        }
    }
    info!(
        "Fog4 live stage{stage}: view_dist_scale{}; next stage waits for validated readback",
        config.view_dist_scale
    );
}

fn setup(
    mut commands: Commands,
    args: Res<aa_dev::DevArgs>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    let mut xml = aa_pak::Pak::global()
        .read_string("worlds/main_world/cells/021_008/client/entities.xml")
        .unwrap();
    if flag("--legacy-control") {
        xml = xml.replace("ViewDistRatio=\"10\"", "ViewDistRatio=\"100\"");
    }
    let volume = fog::parse(&xml)
        .into_iter()
        .find(|v| v.guid == 0x4133521fcc7b527e)
        .unwrap();
    assert_eq!(volume.name, "FogVolume_4");
    assert!(volume.active && !volume.hidden && volume.box_shape && !volume.global_color);
    assert_eq!(volume.size, Vec3::new(60.0, 60.0, 40.0));
    assert_eq!(volume.density, 0.01);
    assert_eq!(volume.color, Vec3::new(0.027451, 0.0352941, 0.0392157));
    let exposure = Exposure { ev100: 0.0 };
    let params = volume.params(CELL, exposure.exposure().recip());
    let center = params.center.truncate();
    let origin = aa_world::cry_to_bevy(Vec3::new(21.0 * 1024.0, 8.0 * 1024.0, 0.0));
    let mesh = meshes.add(fog::mesh(&volume));
    let material = materials.add(FogMaterial::new(&volume, CELL, exposure.exposure().recip()));
    let node = commands
        .spawn((
            Name::new("Actual cell021_008 Fog4"),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(origin),
            NoAutomaticBatching,
        ))
        .id();
    // A real opaque depth receiver behind the original volume; no fog density,
    // colour, size or transform changes are used to manufacture the failure.
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(2000.0, 2000.0))),
        MeshMaterial3d(standards.add(StandardMaterial {
            base_color: Color::linear_rgb(BACK.x, BACK.y, BACK.z),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(center + Vec3::NEG_Z * 100.0),
    ));
    // Final world-coordinate float AABB of this 60x60x40 authored node. Native
    // rotation is ~90 degrees about Z and the XY extents are equal. Their tiny
    // off-axis change is below the world-position ULP at this actual cell.
    let bounds = [
        center - Vec3::new(30.0, 20.0, 30.0),
        center + Vec3::new(30.0, 20.0, 30.0),
    ];
    let distances = if flag("--fov-control") {
        [400.0, 400.0]
    } else {
        [220.0, 450.0]
    };
    let fovs = if flag("--fov-control") {
        [30.0_f32.to_radians(), 80.0_f32.to_radians()]
    } else {
        [50.0_f32.to_radians(); 2]
    };
    let eyes = distances.map(|d| center + Vec3::Z * d);
    let limit = if flag("--legacy-control") {
        2814.249_3
    } else {
        NATIVE_LIMIT
    };
    let expected = std::array::from_fn(|i| native_admitted(bounds, eyes[i], fovs[i], limit));
    assert_eq!(
        expected,
        if flag("--legacy-control") {
            [true, true]
        } else {
            [true, false]
        }
    );
    for i in 0..2 {
        commands.spawn((
            Camera3d::default(),
            Camera {
                order: i as isize,
                viewport: Some(Viewport {
                    physical_position: UVec2::new(i as u32 * args.size.0 / 2, 0),
                    physical_size: UVec2::new(args.size.0 / 2, args.size.1),
                    ..default()
                }),
                clear_color: if i == 0 {
                    default()
                } else {
                    ClearColorConfig::None
                },
                ..default()
            },
            Projection::Perspective(PerspectiveProjection {
                fov: fovs[i],
                near: 0.1,
                far: 2000.0,
                ..default()
            }),
            Transform::from_translation(eyes[i]).looking_to(Vec3::NEG_Z, Vec3::Y),
            DepthPrepass,
            exposure,
            Tonemapping::None,
            DebandDither::Disabled,
            Msaa::Off,
        ));
        info!(
            "Native Fog4 view{i}: eye{:?}, FOV{}deg, AABB{bounds:?}, limit{limit}, admitted{}",
            eyes[i],
            fovs[i].to_degrees(),
            expected[i]
        );
    }
    commands.spawn((Label, Text::new(format!(
        "Original cell021_008 Fog4 / unchanged authored density, colour, rotation and size\nLeft native DRAW / right native {} | {}\nEach centre pixel compared with independent integration and its own same-view background pixel.",
        if expected[1] { "DRAW (explicit legacy100 control)" } else { "CULL" },
        if flag("--fov-control") { "same distance / actual FOV30 vs80" } else { "actual FOV50 / near vs far" })),
        TextFont { font_size: FontSize::Px(17.0), ..default() },
        Node { position_type: PositionType::Absolute, left: px(12.0), top: px(12.0), ..default() }));
    commands.insert_resource(Control {
        volume,
        eyes,
        fovs,
        bounds,
        expected,
        node,
        mesh: mesh.id(),
        material: material.id(),
    });
}

// Independent expectation from the pinned original consumer interval. This
// example's helper is not the production renderer decision under test.
fn native_admitted(bounds: [Vec3; 2], eye: Vec3, fov: f32, limit: f32) -> bool {
    let d = (bounds[0] - eye).max(Vec3::ZERO).max(eye - bounds[1]);
    let distance = ((d.x * d.x + d.y * d.y) + d.z * d.z).sqrt();
    let scale = ((fov * 57.295776 * 0.016666668) * 0.8) + 0.2;
    distance * scale < limit
}

fn display(rgb: Vec3) -> [u8; 3] {
    rgb.to_array().map(|v| {
        ((if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }) * 255.0)
            .round() as u8
    })
}

fn pixels(
    shot: On<ScreenshotCaptured>,
    c: Res<Control>,
    capture: Res<Capture>,
    requests: Query<&RequestStage>,
    parts: Query<(&Mesh3d, &MeshMaterial3d<FogMaterial>, &Transform)>,
    materials: Res<Assets<FogMaterial>>,
    meshes: Res<Assets<Mesh>>,
    config: Res<aa_world::WorldConfig>,
    mut state: ResMut<Cycle>,
) {
    let Ok(RequestStage(requested_stage)) = requests.get(shot.entity) else {
        return;
    };
    assert_eq!(
        *requested_stage, state.stage,
        "capture must belong to the current held stage"
    );
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    // Preserve the evidence before a deliberate baseline assertion aborts; no
    // early AppExit success can hide a failed actual-material readback.
    let path = &capture.output;
    let saved = if capture.cycle {
        let base = path.as_path();
        base.with_file_name(format!(
            "{}-readback{}-phase{}.png",
            base.file_stem().unwrap().to_string_lossy(),
            state.readbacks,
            state.stage
        ))
    } else {
        path.clone()
    };
    if let Some(parent) = saved.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    image.save(&saved).unwrap();
    state.readbacks += 1;
    let unloaded = capture.cycle && state.stage == 4;
    if unloaded {
        assert!(parts.get(c.node).is_err());
        assert!(!meshes.contains(c.mesh) && !materials.contains(c.material));
    } else {
        let (mesh, material, _) = parts.get(c.node).unwrap();
        assert_eq!((mesh.id(), material.id()), (c.mesh, c.material));
        assert!(meshes.contains(c.mesh) && materials.contains(c.material));
    }
    // Original CPU oracle supplies general6/60/120 controls. These cycle points
    // deliberately stay away from equality; exact adjacent-f32 gates are CPU tests.
    let limit = match (flag("--legacy-control"), state.stage, capture.cycle) {
        (false, 1, true) => 28.142_494,
        (false, 2, true) => 562.849_85,
        (true, 1, true) => NATIVE_LIMIT,
        (true, 2, true) => 5628.498_5,
        (true, _, _) => 2814.249_3,
        _ => NATIVE_LIMIT,
    };
    let expected_views: [bool; 2] = if unloaded {
        [false, false]
    } else {
        std::array::from_fn(|i| native_admitted(c.bounds, c.eyes[i], c.fovs[i], limit))
    };
    if !capture.cycle {
        assert_eq!(expected_views, c.expected);
    }
    assert_eq!(
        config.view_dist_scale,
        if capture.cycle && state.stage == 1 {
            0.15
        } else if capture.cycle && state.stage == 2 {
            3.0
        } else {
            1.5
        }
    );
    let (w, h) = image.dimensions();
    let mut failures = Vec::new();
    for i in 0..2 {
        let view_width = w / 2;
        let x = view_width * i as u32 + view_width / 2;
        let y = h / 2;
        let background = image
            .get_pixel(view_width * i as u32 + view_width * 9 / 10, y)
            .0;
        assert!(
            background
                .iter()
                .zip(display(BACK))
                .all(|(a, b)| a.abs_diff(b) <= 2),
            "view{i} needs a valid same-view opaque background control: {background:?}"
        );
        let ndc = Vec2::new((view_width / 2) as f32 + 0.5, y as f32 + 0.5)
            / Vec2::new(view_width as f32, h as f32)
            * 2.0
            - Vec2::ONE;
        let t = (c.fovs[i] * 0.5).tan();
        let ray = Vec3::new(ndc.x * view_width as f32 / h as f32 * t, -ndc.y * t, -1.0).normalize();
        let params = c.volume.params(CELL, 1.0);
        let end = (params.center.z - 100.0 - c.eyes[i].z) / ray.z;
        let dt = f64::from(end) / 100_000.0;
        let mut integral = 0.0_f64;
        for step in 0..100_000 {
            let at = c.eyes[i] + ray * ((step as f64 + 0.5) * dt) as f32;
            let local = params
                .local_from_world
                .transform_vector3(at - params.center.truncate());
            if local.abs().max_element() <= 1.0 {
                integral += dt;
            }
        }
        let alpha = if expected_views[i] {
            (1.0 - (-f64::from(c.volume.density) * integral).exp()) as f32
        } else {
            0.0
        };
        let expected = display(BACK.lerp(c.volume.color, alpha));
        let gpu = image.get_pixel(x, y).0;
        info!(
            "Fog4 actual readback view{i}: GPU{gpu:?} same-view background{background:?} native{expected:?} admitted{} integral{integral} alpha{alpha} bounds{:?}",
            expected_views[i], c.bounds
        );
        if !gpu.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2) {
            failures.push(format!(
                "view{i}: GPU{gpu:?}, native{expected:?}, admitted{}",
                expected_views[i]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "actual Fog4 per-view distance RED: {}",
        failures.join("; ")
    );
    assert!(
        state.acknowledge(*requested_stage),
        "only the outstanding actual readback may validate its stage"
    );
    if capture.cycle && state.stage == 4 {
        assert_eq!(
            state.validated, 31,
            "all earlier cycle readbacks must pass before exit"
        );
    }
    info!(
        "Native per-render-view authored Fog4 material readback PASSED, node and asset handles retained"
    );
}
