//! Original cloud shader / native water-half controls. --ocean 80 --eye 90 --two-views
//! exercises opposite halves; --late-control / --no-ocean / --lose-ocean / --unload-check.
use aa_world::sky_clouds::{CloudMaterial, CloudMaterialPlugin, CloudParams};
use aa_world::sky_material::{
    SkyMaterial, SkyMaterialPlugin, SkyParams, composite::cloud_water::CloudWaterPass,
};
use bevy::{
    asset::{AssetId, embedded_asset},
    camera::visibility::{NoFrustumCulling, RenderLayers},
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin},
    prelude::*,
    render::{
        render_resource::{
            AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        },
        view::screenshot::ScreenshotCaptured,
    },
    shader::ShaderRef,
};

const SKY: Vec3 = Vec3::new(0.12, 0.18, 0.23);
fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}
fn number(name: &str, fallback: f32) -> f32 {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if arg == name {
            return args.next().unwrap().parse().unwrap();
        }
    }
    fallback
}
#[derive(Clone, Copy, ShaderType)]
struct Control {
    color: Vec4,
    settings: Vec4,
}
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct CaptureMaterial {
    #[uniform(0)]
    params: Control,
}
impl Material for CaptureMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://cloud_water/sky_composite_controls.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
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
        d.fragment
            .as_mut()
            .unwrap()
            .shader_defs
            .push("CAPTURE_CONTROL".into());
        Ok(())
    }
}
#[derive(Resource)]
struct Controls(Vec<Vec<(Vec2, [u8; 3])>>);
#[derive(Resource, Default)]
struct State {
    changed: bool,
    verified: bool,
    ids: Vec<AssetId<CloudMaterial>>,
    last: f32,
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native cloud water halves"),
        aa_dev::DevPlugin,
    ));
    embedded_asset!(app, "examples", "sky_composite_controls.wesl");
    app.add_plugins((
        SkyMaterialPlugin,
        CloudMaterialPlugin,
        MaterialPlugin::<CaptureMaterial>::default(),
    ))
    .insert_resource(ClearColor(Color::linear_rgb(0.8, 0.05, 0.8)))
    .init_resource::<State>()
    .add_systems(Startup, setup)
    .add_systems(Update, cleanup)
    .add_systems(Last, force_late.run_if(|| flag("--late-control")))
    .add_observer(pixels)
    .run();
}
fn pose(point: Vec2, y: f32) -> Transform {
    Transform::from_xyz(point.x, y, point.y)
        .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
}
fn display(rgb: Vec3) -> [u8; 3] {
    rgb.to_array().map(|x| {
        ((if x <= 0.0031308 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        }) * 255.0)
            .round() as u8
    })
}
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut skies: ResMut<Assets<SkyMaterial>>,
    mut clouds: ResMut<Assets<CloudMaterial>>,
    mut captures: ResMut<Assets<CaptureMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window>,
    mut state: ResMut<State>,
) {
    let ocean = number("--ocean", 100.0);
    let eye = number("--eye", 150.0);
    let views = if flag("--two-views") { 2 } else { 1 };
    let mut expected = vec![Vec::new(); views];
    if !flag("--no-ocean") {
        commands.spawn((
            aa_world::sky::OceanPlane,
            Transform::from_xyz(0.0, ocean, 0.0),
        ));
        if flag("--ambiguous-ocean") {
            commands.spawn((
                aa_world::sky::OceanPlane,
                Transform::from_xyz(0.0, ocean + 1.0, 0.0),
            ));
        }
    }
    commands.spawn((
        aa_world::sky::SkyDome,
        Mesh3d(meshes.add(Rectangle::new(16.0, 12.0))),
        MeshMaterial3d(skies.add(SkyMaterial {
            params: SkyParams {
                horizon: SKY.extend(1.0),
                zenith: SKY.extend(0.0),
                ..default()
            },
            box_12: None,
            box_34: None,
            box_5: None,
        })),
        pose(Vec2::ZERO, eye + 80.0),
        NoFrustumCulling,
    ));
    let mesh = meshes.add(Rectangle::new(1.5, 1.5));
    let bar_mesh = meshes.add(Rectangle::new(1.5, 0.14));
    let capture = captures.add(CaptureMaterial {
        params: Control {
            color: Vec4::ZERO,
            settings: Vec4::ZERO,
        },
    });
    let textures: Vec<_> = [0_u8, 64, 192, 255]
        .into_iter()
        .map(|red| {
            images.add(Image::new_fill(
                bevy::render::render_resource::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                bevy::render::render_resource::TextureDimension::D2,
                &[red, red, red, 255],
                bevy::render::render_resource::TextureFormat::Rgba8Unorm,
                bevy::asset::RenderAssetUsages::default(),
            ))
        })
        .collect();
    let shade_sky = Vec3::new(0.04, 0.08, 0.12);
    let shade_sun = Vec3::new(0.2, 0.1, 0.04);
    let diffuse = Vec3::new(0.9, 0.8, 0.7);
    for (row, opacity) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        for (column, red) in [0_u8, 64, 192, 255].into_iter().enumerate() {
            let step = if views == 2 { 1.75 } else { 2.0 };
            let point = Vec2::new((column as f32 - 1.5) * step, 2.1 - row as f32 * 2.1);
            let material = clouds.add(CloudMaterial {
                params: CloudParams {
                    shade_sky: shade_sky.extend(1.0),
                    shade_sun: shade_sun.extend(1.0),
                    material: diffuse.extend(opacity),
                    shape: Vec4::new(0.6, 0.0, 1.4, 0.0),
                    sun: Vec4::Y,
                },
                texture: Some(textures[column].clone()),
            });
            state.ids.push(material.id());
            commands.spawn((
                CloudWaterPass,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                pose(point, eye + 50.0),
                NoFrustumCulling,
            ));
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(capture.clone()),
                pose(point, eye + 20.0),
            ));
            // Independent original-shader inputs: linear red density and eight constant taps.
            let density = red as f32 / 255.0;
            let cloud_rgb = shade_sky.lerp(shade_sun, (-0.6_f32 * 8.0 * density).exp2()) * diffuse;
            let alpha = density.powf(1.4) * opacity;
            for view in 0..views {
                let camera_height = eye - view as f32 * 20.0;
                // Native x86 oracle, not the production classifier function.
                let native_after = ((camera_height - ocean) * 0.5) * (100.0 - ocean) > 0.0;
                let before = !native_after
                    && !flag("--late-control")
                    && !flag("--no-ocean")
                    && !flag("--ambiguous-ocean");
                let rgb = if before {
                    SKY.lerp(cloud_rgb, alpha)
                } else {
                    SKY
                };
                expected[view].push((point, display(rgb)));
                commands.spawn((
                    Mesh3d(bar_mesh.clone()),
                    MeshMaterial3d(standards.add(StandardMaterial {
                        base_color: Color::linear_rgb(rgb.x, rgb.y, rgb.z),
                        unlit: true,
                        ..default()
                    })),
                    pose(point - Vec2::Y * 0.95, eye + 10.0),
                    RenderLayers::layer(view + 1),
                ));
            }
        }
    }
    commands.insert_resource(Controls(expected));
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
                    physical_position: UVec2::new(view as u32 * size.x / 2, 0),
                    physical_size: UVec2::new(size.x / 2, size.y),
                    ..default()
                }),
                ..default()
            },
            Transform::from_xyz(0.0, eye - view as f32 * 20.0, 0.0).looking_to(Vec3::Y, Vec3::Z),
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                    viewport_height: 8.0,
                },
                ..OrthographicProjection::default_3d()
            }),
            RenderLayers::from_layers(&[0, view + 1]),
            Tonemapping::None,
            DebandDither::Disabled,
            Msaa::Off,
        ));
    }
    commands.spawn((Text::new(format!("Original cloud shader through real ViewTransmissionTexture; CPU swatches below\nStored native height100, ocean{ocean}; camera{eye}{}; forced-late={}\nDensity bytes0/64/192/255; authored-opacity controls0/.5/1",if views==2 {format!(" / {}",eye-20.0)} else {String::new()},flag("--late-control"))),TextFont{font_size:FontSize::Px(18.0),..default()},Node{position_type:PositionType::Absolute,left:px(15.0),top:px(10.0),..default()}));
}
fn force_late(mut commands: Commands, clouds: Query<Entity, With<CloudWaterPass>>) {
    for entity in &clouds {
        commands.entity(entity).remove::<CloudWaterPass>();
    }
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>, state: Res<State>) {
    if flag("--lose-ocean") || flag("--unload-check") {
        assert!(state.verified, "capture must follow cleanup verification");
    }
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (view, cases) in controls.0.iter().enumerate() {
        for (i, (point, expected)) in cases.iter().enumerate() {
            let x = (w as f32 * (view as f32 + 0.5) / controls.0.len() as f32
                + point.x * h as f32 / 8.0) as u32;
            let y = (h as f32 * 0.5 - point.y * h as f32 / 8.0) as u32;
            let gpu = image.get_pixel(x, y).0;
            let cpu = if state.changed {
                display(SKY)
            } else {
                *expected
            };
            assert!(
                gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(b) <= 2),
                "Cloud half view{view} case{i}:GPU{gpu:?} CPU{cpu:?}"
            );
            info!(
                "Native cloud water GPU view{view} case{i} PASSED GPU{gpu:?} independentCPU{cpu:?}"
            );
        }
    }
}
fn cleanup(
    time: Res<Time>,
    mut commands: Commands,
    mut state: ResMut<State>,
    sources: Query<Entity, With<MeshMaterial3d<CloudMaterial>>>,
    oceans: Query<Entity, With<aa_world::sky::OceanPlane>>,
    clouds: Res<Assets<CloudMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    let now = time.elapsed_secs();
    if now - state.last > 2.0 {
        state.last = now;
        info!(
            "cloud water sources={} materials={}",
            sources.iter().count(),
            clouds.len()
        );
    }
    if now > 12.0 && !state.changed && (flag("--lose-ocean") || flag("--unload-check")) {
        if flag("--lose-ocean") {
            for entity in &oceans {
                commands.entity(entity).despawn();
            }
        } else {
            for entity in &sources {
                commands.entity(entity).despawn();
            }
        }
        for (_, material) in standards.iter_mut() {
            material.base_color = Color::linear_rgb(SKY.x, SKY.y, SKY.z);
        }
        state.changed = true;
    }
    if now > 16.0 && state.changed && !state.verified {
        if flag("--lose-ocean") {
            assert_eq!(sources.iter().count(), 12);
            assert_eq!(clouds.len(), 12);
            assert!(oceans.is_empty());
        } else {
            assert!(sources.is_empty());
            assert!(state.ids.iter().all(|id| !clouds.contains(*id)));
            assert!(clouds.is_empty());
        }
        state.verified = true;
        info!(
            "Native cloud water cleanup PASSED: ocean_loss={} sources={} materials={}",
            flag("--lose-ocean"),
            sources.iter().count(),
            clouds.len()
        );
    }
}
