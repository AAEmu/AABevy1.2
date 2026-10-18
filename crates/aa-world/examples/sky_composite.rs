//! Native sky pass order: GPU blend/depth controls or original forest --world [--night] [--unload-check].
use aa_world::sky_material::{
    SkyMaterial, SkyMaterialPlugin, SkyParams,
    composite::{SkyComposite, SkyCompositeMaterial, SkyPass},
};
use bevy::{
    asset::{AssetId, embedded_asset},
    camera::visibility::NoFrustumCulling,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin},
    prelude::*,
    render::{
        render_resource::{
            AsBindGroup, CompareFunction, RenderPipelineDescriptor, ShaderType,
            SpecializedMeshPipelineError,
        },
        view::screenshot::ScreenshotCaptured,
    },
    shader::ShaderRef,
};

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}
#[derive(Clone, Copy, ShaderType)]
struct Control {
    color: Vec4,
    settings: Vec4,
}
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct ControlMaterial {
    #[uniform(0)]
    params: Control,
    mode: AlphaMode,
    sort: f32,
}
impl Material for ControlMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://sky_composite/sky_composite_controls.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        self.mode
    }
    fn depth_bias(&self) -> f32 {
        self.sort
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _: &MaterialPipeline,
        d: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        d.primitive.cull_mode = None;
        if let Some(depth) = &mut d.depth_stencil {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
        }
        Ok(())
    }
}
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct CaptureMaterial {
    #[uniform(0)]
    params: Control,
}
impl Material for CaptureMaterial {
    fn vertex_shader() -> ShaderRef {
        ControlMaterial::vertex_shader()
    }
    fn fragment_shader() -> ShaderRef {
        ControlMaterial::fragment_shader()
    }
    fn reads_view_transmission_texture(&self) -> bool {
        true
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _: &MaterialPipeline,
        d: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        d.primitive.cull_mode = None;
        if let Some(fragment) = &mut d.fragment {
            fragment.shader_defs.push("CAPTURE_CONTROL".into());
        }
        Ok(())
    }
}
#[derive(Resource)]
struct Controls(Vec<(Vec2, [u8; 3])>);
#[derive(Resource, Default)]
struct Scene {
    at: f32,
    removed: bool,
    verified: bool,
    children: Vec<Entity>,
    materials: Vec<AssetId<SkyCompositeMaterial>>,
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native moon / atmosphere composition"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = 180.;
        config.unload_radius = 350.;
        config.objects = false;
        config.vegetation = false;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: if flag("--night") { 0. } else { 10. },
                ..default()
            })
            .add_systems(Startup, world_setup)
            .add_systems(Update, world_stats);
    } else {
        embedded_asset!(app, "examples", "sky_composite_controls.wesl");
        app.add_plugins((
            SkyMaterialPlugin,
            MaterialPlugin::<ControlMaterial>::default(),
            MaterialPlugin::<CaptureMaterial>::default(),
        ))
        .insert_resource(ClearColor(Color::linear_rgb(0.8, 0.05, 0.8)))
        .add_systems(Startup, numeric)
        .add_observer(pixels);
        app.sub_app_mut(bevy::render::RenderApp).add_systems(
            bevy::core_pipeline::schedule::Core3d,
            render_controls
                .after(bevy::core_pipeline::core_3d::main_transparent_pass_3d)
                .in_set(bevy::core_pipeline::Core3dSystems::MainPass),
        );
    }
    app.init_resource::<Scene>()
        .add_systems(Update, lifetime)
        .add_systems(Last, force_late.run_if(|| flag("--late-control")))
        .run();
}
fn render_controls(
    view: bevy::render::renderer::ViewQuery<(
        &bevy::render::view::ExtractedView,
        Option<&bevy::pbr::ViewTransmissionTexture>,
    )>,
    phases: Res<
        bevy::render::render_phase::ViewSortedRenderPhases<
            bevy::core_pipeline::core_3d::Transparent3d,
        >,
    >,
    capture: Res<bevy::render::render_phase::ViewSortedRenderPhases<bevy::pbr::Transmissive3d>>,
    tagged: Query<&bevy::render::sync_world::MainEntity, With<SkyPass>>,
    mut reported: Local<bool>,
) {
    let (view, texture) = view.into_inner();
    let Some(phase) = phases.get(&view.retained_view_entity) else {
        return;
    };
    if *reported || phase.items.is_empty() {
        return;
    }
    let sky: Vec<_> = phase
        .items
        .values()
        .filter(|item| tagged.iter().any(|main| *main == item.entity.1))
        .collect();
    *reported = true;
    info!(
        "Sky render control: tagged={} queued={} suppressed={} transmission_texture={} capture_items={}",
        tagged.iter().count(),
        sky.len(),
        sky.iter()
            .filter(|item| item.batch_range.is_empty())
            .count(),
        texture.is_some(),
        capture
            .get(&view.retained_view_entity)
            .map_or(0, |phase| phase.items.len())
    );
}
fn force_late(mut commands: Commands, sky: Query<Entity, With<SkyPass>>) {
    for entity in &sky {
        commands.entity(entity).remove::<SkyPass>();
    }
}

fn world_setup(mut commands: Commands) {
    let eye = aa_world::cry_to_bevy(Vec3::new(12842.47, 14535.70, 172.27849));
    let target = aa_world::cry_to_bevy(Vec3::new(12812.47, 14575.70, 158.27849));
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(eye).looking_at(target, Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 50_f32.to_radians(),
            near: 0.1,
            far: 20000.,
            ..default()
        }),
    ));
}
fn world_stats(
    time: Res<Time>,
    env: Res<aa_world::sky::Environment>,
    moons: Res<aa_world::moons::MoonStats>,
    mut last: Local<f32>,
) {
    if time.elapsed_secs() - *last > 2. {
        *last = time.elapsed_secs();
        info!(
            "Sky composition original forest: zone={:?} hour={} moons={}/{} primary={:?}",
            env.zone,
            env.hour,
            moons.rendered,
            moons.authored,
            env.sample.as_ref().map(|s| s.moons[0].color)
        );
    }
}
fn display(rgb: Vec3) -> [u8; 3] {
    rgb.to_array().map(|x| {
        ((if x <= 0.0031308 {
            12.92 * x
        } else {
            1.055 * x.powf(1. / 2.4) - 0.055
        }) * 255.)
            .round() as u8
    })
}
fn pose(point: Vec2, depth: f32) -> Transform {
    Transform::from_xyz(point.x, depth, point.y)
        .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
}
fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut skies: ResMut<Assets<SkyMaterial>>,
    mut controls: ResMut<Assets<ControlMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut captures: ResMut<Assets<CaptureMaterial>>,
    windows: Query<&Window>,
) {
    let white = images.add(Image::new_fill(
        bevy::render::render_resource::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        &[255; 4],
        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::default(),
    ));
    let mesh = meshes.add(Rectangle::new(1.6, 1.6));
    let atmosphere = Vec3::new(0.12, 0.18, 0.23);
    let cloud = Vec3::new(0.35, 0.22, 0.09);
    let stars = Vec3::new(0.04, 0.08, 0.12);
    let moon = Vec3::new(0.08, 0.05, 0.02);
    let moon2 = Vec3::new(0.03, 0.07, 0.11);
    let mut expected = Vec::new();
    for (row, alpha) in [0., 0.5, 1.].into_iter().enumerate() {
        for column in 0..4 {
            let step = if flag("--two-views") { 1.75 } else { 2. };
            let point = Vec2::new((column as f32 - 1.5) * step, 2.1 - row as f32 * 2.1);
            let params = SkyParams {
                horizon: atmosphere.extend(1.),
                zenith: atmosphere.extend(0.),
                box_sky: cloud.extend(alpha),
                box_sun: cloud.extend(0.),
                box_params: Vec4::new(1., 1., 0., 1.),
                ..default()
            };
            commands.spawn((
                aa_world::sky::SkyDome,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(skies.add(SkyMaterial {
                    params,
                    box_12: Some(white.clone()),
                    box_34: Some(white.clone()),
                    box_5: Some(white.clone()),
                })),
                pose(point, 10.),
                NoFrustumCulling,
            ));
            let mut celestial = Vec3::ZERO;
            let mut layer = |color: Vec4, mode, sort| {
                let material = controls.add(ControlMaterial {
                    params: Control {
                        color,
                        settings: Vec4::X,
                    },
                    mode,
                    sort,
                });
                commands.spawn((
                    SkyPass,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    pose(point, 10.),
                    NoFrustumCulling,
                ));
            };
            if column > 0 {
                layer(stars.extend(0.), AlphaMode::Add, -1_000_000.);
                celestial = stars;
                if column == 1 {
                    layer(Vec4::W, AlphaMode::Blend, -999_999.);
                    celestial = Vec3::ZERO;
                }
                if column >= 2 {
                    layer(moon.extend(0.75), AlphaMode::Blend, -999_999.);
                    celestial = moon * 0.75 + celestial * 0.25;
                }
                if column == 3 {
                    layer(moon2.extend(0.5), AlphaMode::Blend, -999_998.);
                    celestial = moon2 * 0.5 + celestial * 0.5;
                }
            }
            let mut native = (celestial + atmosphere) * (1. - alpha) + cloud * alpha;
            if flag("--transmission") {
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(captures.add(CaptureMaterial {
                        params: Control {
                            color: Vec4::ZERO,
                            settings: Vec4::ZERO,
                        },
                    })),
                    pose(point, 5.),
                ));
                if flag("--late-control") {
                    native = Vec3::ZERO;
                }
            }
            if row == 1 && column == 2 {
                let glass = Vec3::new(0.02, 0.1, 0.03);
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(controls.add(ControlMaterial {
                        params: Control {
                            color: glass.extend(0.5),
                            settings: Vec4::ZERO,
                        },
                        mode: AlphaMode::Blend,
                        sort: 0.,
                    })),
                    pose(point, 5.),
                ));
                native = glass * 0.5 + native * 0.5;
            }
            if row == 1 && column == 3 {
                native = Vec3::new(0.16, 0.07, 0.025);
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(standards.add(StandardMaterial {
                        base_color: Color::linear_rgb(native.x, native.y, native.z),
                        unlit: true,
                        ..default()
                    })),
                    pose(point, 5.),
                ));
            }
            expected.push((point, display(native)));
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(1.6, 0.14))),
                MeshMaterial3d(standards.add(StandardMaterial {
                    base_color: Color::linear_rgb(native.x, native.y, native.z),
                    unlit: true,
                    ..default()
                })),
                pose(point - Vec2::Y * 0.95, 4.),
            ));
        }
    }
    commands.insert_resource(Controls(expected));
    let views = if flag("--two-views") { 2 } else { 1 };
    let size = windows.single().unwrap().physical_size();
    for view in 0..views {
        commands.spawn((
            Camera3d::default(),
            bevy::pbr::ScreenSpaceTransmission::default(),
            Camera {
                order: view as isize,
                clear_color: if view == 0 {
                    default()
                } else {
                    ClearColorConfig::None
                },
                viewport: (views == 2).then_some(bevy::camera::Viewport {
                    physical_position: UVec2::new(view * size.x / 2, 0),
                    physical_size: UVec2::new(size.x / 2, size.y),
                    ..default()
                }),
                ..default()
            },
            Transform::default().looking_to(Vec3::Y, Vec3::Z),
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                    viewport_height: 8.,
                },
                ..OrthographicProjection::default_3d()
            }),
            Tonemapping::None,
            DebandDither::Disabled,
            Msaa::Off,
        ));
    }
    commands.spawn((Text::new(format!("Native sky composition; independent CPU swatches below each GPU panel\nColumns: empty / opaque dark Moon + stars / lit Moon + stars / overlapping Moons\nRows: cloud opacity 0 / .5 / 1. Middle last two: foreground glass / opaque depth receiver.\nViewTransmissionTexture capture={} forced late control={}",flag("--transmission"),flag("--late-control"))),TextFont{font_size:FontSize::Px(18.),..default()},Node{position_type:PositionType::Absolute,left:px(15.),top:px(10.),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>, scene: Res<Scene>) {
    if scene.removed {
        assert!(scene.verified, "screenshot must follow the cleanup check");
        return;
    }
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    let views = if flag("--two-views") { 2 } else { 1 };
    for view in 0..views {
        for (i, (point, cpu)) in controls.0.iter().enumerate() {
            let x =
                (w as f32 * (view as f32 + 0.5) / views as f32 + point.x * h as f32 / 8.) as u32;
            let y = (h as f32 * 0.5 - point.y * h as f32 / 8.) as u32;
            let gpu = image.get_pixel(x, y).0;
            assert!(
                gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= 2),
                "Sky composition{i} GPU{gpu:?} CPU{cpu:?}"
            );
            info!(
                "Native sky composition GPU view{view} case{i} PASSED GPU{gpu:?} independentCPU{cpu:?}"
            );
        }
    }
}
fn lifetime(
    time: Res<Time>,
    mut commands: Commands,
    sources: Query<Entity, With<aa_world::sky::SkyDome>>,
    children: Query<(Entity, &MeshMaterial3d<SkyCompositeMaterial>), With<SkyComposite>>,
    all: Query<()>,
    materials: Res<Assets<SkyCompositeMaterial>>,
    mut scene: ResMut<Scene>,
) {
    let now = time.elapsed_secs();
    if now - scene.at > 2. {
        scene.at = now;
        info!(
            "Sky composition source={} late={} material={}",
            sources.iter().count(),
            children.iter().count(),
            materials.len()
        );
    }
    if flag("--unload-check") && !scene.removed && now > 16. {
        assert!(!sources.is_empty());
        assert_eq!(sources.iter().count(), children.iter().count());
        for (entity, handle) in &children {
            scene.children.push(entity);
            scene.materials.push(handle.id());
        }
        for source in &sources {
            commands.entity(source).despawn();
        }
        scene.removed = true;
    }
    if scene.removed && !scene.verified && now > 19. {
        assert!(children.is_empty());
        assert!(materials.is_empty());
        assert!(scene.children.iter().all(|e| all.get(*e).is_err()));
        assert!(scene.materials.iter().all(|m| !materials.contains(*m)));
        scene.verified = true;
        info!(
            "Native sky composition unload PASSED: {} child entities / {} materials gone",
            scene.children.len(),
            scene.materials.len()
        );
    }
}
