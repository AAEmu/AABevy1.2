//! Native fog-water halves through an actual ViewTransmissionTexture.
//! --two-views --ellipsoid; --late-control; --crossing / --lose-ocean / --unload-check (20 s).
use aa_world::fog::{self, FogMaterial, FogVolume, FogVolumePlugin, FogWaterPass};
use bevy::{
    asset::{AssetId, embedded_asset},
    camera::{Exposure, visibility::RenderLayers},
    core_pipeline::{
        prepass::DepthPrepass,
        tonemapping::{DebandDither, Tonemapping},
    },
    mesh::MeshVertexBufferLayoutRef,
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin},
    prelude::*,
    render::{
        render_resource::{AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError},
        view::screenshot::ScreenshotCaptured,
    },
    shader::ShaderRef,
};
const BACK: Vec3 = Vec3::new(0.12, 0.24, 0.4);
const COLOR: Vec3 = Vec3::new(0.7, 0.4, 0.2);
const DECAL: Vec3 = Vec3::new(0.15, 0.7, 0.2);
const FOV: f32 = 50.0;
fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}
fn number(name: &str, default: f32) -> f32 {
    let a: Vec<_> = std::env::args().collect();
    a.iter()
        .position(|s| s == name)
        .and_then(|i| a.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
fn swap(c: Vec3) -> Vec3 {
    Vec3::new(c.z, c.y, c.x)
}
fn display(c: Vec3) -> [u8; 3] {
    c.to_array().map(|x| {
        ((if x <= 0.0031308 {
            x * 12.92
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        }) * 255.0)
            .round() as u8
    })
}
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct CaptureMaterial {
    #[uniform(0)]
    color: Vec4,
}
impl Material for CaptureMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://fog_water/fog_water_controls.wesl".into()
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
        Ok(())
    }
}
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct OverlayMaterial {
    #[uniform(0)]
    color: Vec4,
    road: bool,
}
impl Material for OverlayMaterial {
    fn vertex_shader() -> ShaderRef {
        CaptureMaterial::vertex_shader()
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        if self.road { -600_000.0 } else { -500_000.0 }
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
            depth.bias.constant = 0;
        }
        d.fragment
            .as_mut()
            .unwrap()
            .shader_defs
            .push("OVERLAY_CONTROL".into());
        Ok(())
    }
}
#[derive(Resource)]
struct Control {
    volume: FogVolume,
    eyes: Vec<Vec3>,
    ocean: f32,
    wall: f32,
    size: UVec2,
}
#[derive(Resource, Default)]
struct State {
    changed: bool,
    verified: bool,
    id: Option<AssetId<FogMaterial>>,
    last: f32,
}
#[derive(Component)]
struct CameraIndex(usize);
#[derive(Component)]
struct CardIndex(usize);
#[derive(Component)]
struct Bar(usize);
fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native fog water halves"),
        aa_dev::DevPlugin,
    ));
    embedded_asset!(app, "examples", "fog_water_controls.wesl");
    app.add_plugins((
        aa_world::sky_material::SkyMaterialPlugin,
        FogVolumePlugin,
        MaterialPlugin::<CaptureMaterial>::default(),
        MaterialPlugin::<OverlayMaterial>::default(),
    ))
    .insert_resource(ClearColor(Color::BLACK))
    .init_resource::<State>()
    .add_systems(Startup, setup)
    .add_systems(Update, transition)
    .add_observer(pixels)
    .run();
}

// Independent midpoint density integration, rather than the production analytic
// opacity function. Original fixture has no height gradient or ellipsoid edge fade.
fn expected(c: &Control, index: usize, changed: bool) -> Vec3 {
    let mut eye = c.eyes[index];
    if changed && flag("--crossing") {
        eye.y = 2.0 * c.ocean - eye.y;
    }
    let height = c.size.y as f32;
    let scale = (FOV.to_radians() * 0.5).tan();
    let ray = Vec3::new(scale / height, -scale / height, -1.0).normalize();
    let distance = (c.wall - eye.z) / ray.z;
    let center = aa_world::cry_to_bevy(c.volume.matrix.w_axis.truncate());
    let half = Vec3::new(c.volume.size.x, c.volume.size.z, c.volume.size.y) * 0.5;
    let steps = 100_000;
    let dt = distance / steps as f32;
    let mut path = 0.0_f64;
    for i in 0..steps {
        let p = (eye + ray * ((i as f32 + 0.5) * dt) - center) / half;
        if if c.volume.box_shape {
            p.abs().max_element() <= 1.0
        } else {
            p.length_squared() <= 1.0
        } {
            path += dt as f64;
        }
    }
    let alpha = if flag("--no-depth") || (changed && flag("--unload-check")) {
        0.0
    } else {
        (1.0 - (-(c.volume.density as f64) * path).exp()) as f32
    };
    let native_after = ((eye.y - c.ocean) * 0.5) * (center.y - c.ocean) > 0.0;
    let before = !native_after
        && !flag("--late-control")
        && !flag("--no-ocean")
        && !(changed && flag("--lose-ocean"));
    // Native deferred diffuse is already on the opaque receiver. Forced-late
    // overlays miss the copy and fail depth behind the diagnostic receiver.
    let scene = if (flag("--decal-control") || flag("--road-control") || flag("--forward-control")) && !flag("--late-decal-control") {
        BACK.lerp(DECAL, 0.6)
    } else {
        BACK
    };
    if before {
        swap(scene.lerp(COLOR, alpha))
    } else {
        swap(scene).lerp(COLOR, alpha)
    }
}
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut fogs: ResMut<Assets<FogMaterial>>,
    mut captures: ResMut<Assets<CaptureMaterial>>,
    mut overlays: ResMut<Assets<OverlayMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
    windows: Query<&Window>,
    mut state: ResMut<State>,
) {
    let ocean = number("--ocean", 100.0);
    let height = number("--center", 110.0);
    let xml = format!(
        r#"<Mission><Objects><Entity EntityClass="FogVolume" Pos="0,8,{height}"><Properties VolumeType="{}" GlobalDensity="{}" FallOffScale="0" SoftEdges="0" color_Color="0.7,0.4,0.2"><Size x="200" y="4" z="80"/></Properties></Entity></Objects></Mission>"#,
        if flag("--ellipsoid") { 0 } else { 1 },
        number("--density", 0.2)
    );
    let volume = fog::parse(&xml).remove(0);
    let views = if flag("--two-views") { 2 } else { 1 };
    let eyes = (0..views)
        .map(|i| {
            Vec3::new(
                0.0,
                number("--eye", 90.0) + i as f32 * 20.0,
                if flag("--inside") { -8.0 } else { 0.0 },
            )
        })
        .collect();
    let c = Control {
        volume,
        eyes,
        ocean,
        wall: if flag("--front") {
            -5.0
        } else if flag("--middle") {
            -8.0
        } else {
            -12.0
        },
        size: windows.single().unwrap().physical_size(),
    };
    let exposure = Exposure { ev100: 0.0 };
    let params = c.volume.params((0, 0), exposure.exposure().recip());
    let material = fogs.add(FogMaterial { params });
    state.id = Some(material.id());
    let mut entity = commands.spawn((
        Mesh3d(meshes.add(fog::mesh(&c.volume))),
        MeshMaterial3d(material),
        Transform::default(),
    ));
    if !flag("--late-control") {
        entity.insert(FogWaterPass {
            center_height: params.center.y,
        });
    }
    if !flag("--no-ocean") {
        commands.spawn((
            aa_world::sky::OceanPlane,
            Transform::from_xyz(0.0, ocean, 0.0),
        ));
    }
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(200.0, 200.0))),
        MeshMaterial3d(standards.add(StandardMaterial {
            base_color: Color::linear_rgb(BACK.x, BACK.y, BACK.z),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(0.0, height, c.wall),
    ));
    let capture = captures.add(CaptureMaterial { color: Vec4::ZERO });
    if flag("--decal-control") || flag("--road-control") || flag("--forward-control") {
        let mut decal = commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(200.0, 200.0))),
            MeshMaterial3d(overlays.add(OverlayMaterial {
                color: DECAL.extend(0.6),
                road: flag("--road-control") || flag("--forward-control"),
            })),
            Transform::from_xyz(0.0, height, c.wall + 0.01),
        ));
        if !flag("--late-decal-control") {
            if flag("--road-control") {
                decal.insert(aa_world::sky_material::composite::RoadSurfacePass);
            } else if flag("--forward-control") {
                decal.insert(aa_world::sky_material::composite::ForwardDecalPass);
            } else {
                decal.insert(aa_world::sky_material::composite::DeferredDecalPass);
            }
        }
    }
    let quad = meshes.add(Rectangle::new(100.0, 100.0));
    let bar = meshes.add(Rectangle::new(0.9, 0.16));
    for (i, &eye) in c.eyes.iter().enumerate() {
        commands.spawn((
            CardIndex(i),
            Mesh3d(quad.clone()),
            MeshMaterial3d(capture.clone()),
            Transform::from_xyz(0.0, eye.y, eye.z - 3.0),
            RenderLayers::layer(i + 1),
        ));
        let cpu = expected(&c, i, false);
        commands.spawn((
            Bar(i),
            Mesh3d(bar.clone()),
            MeshMaterial3d(standards.add(StandardMaterial {
                base_color: Color::linear_rgb(cpu.x, cpu.y, cpu.z),
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(0.0, eye.y - 0.7, eye.z - 2.0),
            RenderLayers::layer(i + 1),
        ));
        let mut camera = commands.spawn((
            CameraIndex(i),
            Camera3d::default(),
            bevy::pbr::ScreenSpaceTransmission::default(),
            Camera {
                order: i as isize,
                clear_color: if i == 0 {
                    default()
                } else {
                    ClearColorConfig::None
                },
                viewport: (views == 2).then_some(bevy::camera::Viewport {
                    physical_position: UVec2::new(i as u32 * c.size.x / 2, 0),
                    physical_size: UVec2::new(c.size.x / 2, c.size.y),
                    ..default()
                }),
                ..default()
            },
            Transform::from_translation(eye).looking_to(Vec3::NEG_Z, Vec3::Y),
            Projection::Perspective(PerspectiveProjection {
                fov: FOV.to_radians(),
                near: 0.1,
                far: 1000.0,
                ..default()
            }),
            RenderLayers::from_layers(&[0, i + 1]),
            exposure,
            Tonemapping::None,
            DebandDither::Disabled,
            Msaa::Off,
        ));
        if !flag("--no-depth") {
            camera.insert(DepthPrepass);
        }
    }
    commands.spawn((Text::new(format!("Native FogVolume water halves: real transmission copy reverses RGB; CPU swatches below\nStored centre{height}, ocean{ocean}, eye{}{}; forced-late={}\n--ellipsoid --inside --middle --front --no-depth --crossing --lose-ocean --unload-check",c.eyes[0].y,if views==2{format!(" / {}",c.eyes[1].y)}else{String::new()},flag("--late-control"))),TextFont{font_size:FontSize::Px(18.0),..default()},Node{position_type:PositionType::Absolute,left:px(15.0),top:px(12.0),..default()}));
    commands.insert_resource(c);
}
fn transition(
    time: Res<Time>,
    mut commands: Commands,
    c: Res<Control>,
    mut state: ResMut<State>,
    sources: Query<Entity, With<MeshMaterial3d<FogMaterial>>>,
    oceans: Query<Entity, With<aa_world::sky::OceanPlane>>,
    fogs: Res<Assets<FogMaterial>>,
    mut cameras: Query<(&CameraIndex, &mut Transform), (Without<CardIndex>, Without<Bar>)>,
    mut cards: Query<(&CardIndex, &mut Transform), (Without<CameraIndex>, Without<Bar>)>,
    mut bars: Query<
        (&Bar, &MeshMaterial3d<StandardMaterial>, &mut Transform),
        (Without<CameraIndex>, Without<CardIndex>),
    >,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    let now = time.elapsed_secs();
    if now - state.last > 2.0 {
        state.last = now;
        info!(
            "fog water sources={} materials={}",
            sources.iter().count(),
            fogs.len()
        );
    }
    if now > 12.0
        && !state.changed
        && (flag("--crossing") || flag("--lose-ocean") || flag("--unload-check"))
    {
        if flag("--crossing") {
            for (i, mut t) in &mut cameras {
                t.translation.y = 2.0 * c.ocean - c.eyes[i.0].y;
            }
            for (i, mut t) in &mut cards {
                t.translation.y = 2.0 * c.ocean - c.eyes[i.0].y;
            }
        }
        if flag("--lose-ocean") {
            for e in &oceans {
                commands.entity(e).despawn();
            }
        }
        if flag("--unload-check") {
            for e in &sources {
                commands.entity(e).despawn();
            }
        }
        for (bar, handle, mut transform) in &mut bars {
            let rgb = expected(&c, bar.0, true);
            if flag("--crossing") {
                transform.translation.y = 2.0 * c.ocean - c.eyes[bar.0].y - 0.7;
            }
            standards.get_mut(&handle.0).unwrap().base_color =
                Color::linear_rgb(rgb.x, rgb.y, rgb.z);
        }
        state.changed = true;
    }
    if now > 16.0 && state.changed && !state.verified {
        let count = if flag("--unload-check") { 0 } else { 1 };
        assert_eq!(sources.iter().count(), count);
        assert_eq!(fogs.len(), count);
        if count == 0 {
            assert!(!fogs.contains(state.id.unwrap()));
        }
        if flag("--lose-ocean") {
            assert!(oceans.is_empty());
        }
        state.verified = true;
        info!(
            "Native fog water cleanup/crossing PASSED: sources={count} materials={}",
            fogs.len()
        );
    }
}
fn pixels(shot: On<ScreenshotCaptured>, c: Res<Control>, state: Res<State>) {
    if flag("--crossing") || flag("--lose-ocean") || flag("--unload-check") {
        assert!(
            state.verified,
            "capture must follow 16-second lifecycle check"
        );
    }
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for i in 0..c.eyes.len() {
        let gpu = image
            .get_pixel(w * (2 * i as u32 + 1) / (2 * c.eyes.len() as u32), h / 2)
            .0;
        let cpu = display(expected(&c, i, state.changed));
        assert!(
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(b) <= 2),
            "fog water view{i} GPU{gpu:?} CPU{cpu:?}"
        );
        info!("Native fog water GPU view{i} PASSED GPU{gpu:?} independentCPU{cpu:?}");
    }
}
