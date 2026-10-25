//! Native triangle SID weights at each actual patch step. --world [--baseline] [--unload-check].
use aa_world::terrain_layers::{TerrainExt, TerrainMaterial, TerrainMaterialPlugin, TerrainParams};
use bevy::{
    asset::{AssetId, RenderAssetUsages},
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    image::ImageSampler,
    mesh::VertexAttributeValues,
    prelude::*,
    render::{
        render_resource::{
            Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
        },
        view::screenshot::ScreenshotCaptured,
    },
};
use std::collections::HashSet;
fn flag(s: &str) -> bool {
    std::env::args().any(|a| a == s)
}
#[derive(Resource)]
struct Controls(Vec<(Vec2, [u8; 3])>);
#[derive(Resource, Default)]
struct State {
    mats: HashSet<AssetId<TerrainMaterial>>,
    meshes: HashSet<AssetId<Mesh>>,
    images: HashSet<AssetId<Image>>,
    moved: bool,
    done: bool,
}
fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native terrain triangle weights"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = 180.0;
        config.unload_radius = 350.0;
        config.objects = false;
        config.vegetation = false;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: 10.0,
                ..default()
            })
            .init_resource::<State>()
            .add_systems(Startup, world_camera)
            // Inspect newly spawned CPU meshes after Update's deferred commands and
            // before this frame's render extraction removes their vertex data.
            .add_systems(Last, world_check);
    } else {
        app.add_plugins(TerrainMaterialPlugin)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup, numeric)
            .add_observer(pixels);
    }
    app.run();
}
fn surface(step: usize, merged: bool, images: &mut Assets<Image>) -> Handle<Image> {
    let mut ids = vec![0; 513 * 513];
    ids[step] = 1;
    ids[step * 513] = if merged { 1 } else { 2 };
    ids[step * 513 + step] = if merged { 2 } else { 3 };
    images.add(aa_world::terrain_layers::surface_image(&ids))
}
fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    let colours = [[255u8, 0, 64], [0, 255, 64], [0, 64, 255], [200, 200, 0]];
    let mut bytes = Vec::new();
    for c in colours {
        for _ in 0..16 {
            bytes.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 4,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::nearest();
    let array = images.add(image);
    // Original x86 one-hot interpolation outputs, not a copy of the shader's weight formula.
    let samples = [
        ([0.125, 0.25], [0.625, 0.125, 0.25, 0.0]),
        ([0.5, 0.25], [0.25, 0.5, 0.25, 0.0]),
        ([0.25, 0.5], [0.25, 0.25, 0.5, 0.0]),
        ([0.75, 0.75], [0.0, 0.25, 0.25, 0.5]),
        ([0.5, 0.5], [0.0, 0.5, 0.5, 0.0]),
        ([0.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
        ([0.5, 0.5], [0.0, 1.0, 0.0, 0.0]),
        ([1.0, 0.0], [0.0, 1.0, 0.0, 0.0]),
        ([1.0, 1.0], [0.0, 0.0, 0.0, 1.0]),
    ];
    let cover = [82.0_f64 / 255.0, 85.0 / 255.0, 128.0 / 255.0];
    let r = (cover[0] + 0.001012) * 31.0 / 30.0;
    let gamma = [r, cover[1], 1.0 - r - cover[1]].map(|v| (v * 3.0 * cover[2]).clamp(0.0, 1.0));
    let mut controls = Vec::new();
    for (row, step) in aa_world::terrain::LOD_STEPS.into_iter().enumerate() {
        let normal = surface(step, false, &mut images);
        let merged = surface(step, true, &mut images);
        for (column, (point, weights)) in samples.into_iter().enumerate() {
            let center = Vec2::new(column as f32 * 1.8 - 7.2, 3.7 - row as f32 * 1.8);
            let mut params = TerrainParams::new(Vec3::ONE);
            params.tint.w = 1e9;
            params.detail_distance_z = 1e9;
            for sid in 0..4 {
                let e = (sid * 3 + 2) * 4;
                params.layers[e] = Vec4::new(sid as f32, -1.0, 0.0, 0.0);
                params.layers[e + 1] = Vec4::new(0.0, 0.0, 1.0, 0.0);
            }
            let mut mesh = Mesh::from(Rectangle::new(1.4, 1.25));
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_UV_0,
                vec![
                    [
                        point[1] * step as f32 / 512.0,
                        point[0] * step as f32 / 512.0
                    ];
                    4
                ],
            );
            // First LOD0 control also checks the absent-UV1 fallback used by older fixtures.
            if row != 0 || column != 0 {
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[step as f32, 0.0]; 4]);
            }
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(TerrainMaterial {
                    base: StandardMaterial {
                        base_color: Color::linear_rgb(
                            cover[0] as f32,
                            cover[1] as f32,
                            cover[2] as f32,
                        ),
                        unlit: true,
                        ..default()
                    },
                    extension: TerrainExt::new(
                        params,
                        std::array::from_fn(|_| array.clone()),
                        if column == 6 {
                            merged.clone()
                        } else {
                            normal.clone()
                        },
                    ),
                })),
                Transform::from_xyz(center.x, center.y, 0.0),
            ));
            let mut linear = gamma.map(|v| v.powf(2.2));
            for sid in 0..4 {
                let alpha = (weights[sid] as f64).sqrt();
                for c in 0..3 {
                    let layer = (gamma[c] + colours[sid][c] as f64 / 255.0 - 0.5)
                        .clamp(0.0, 1.0)
                        .powf(2.2);
                    linear[c] = linear[c] * (1.0 - alpha) + layer * alpha;
                }
            }
            let expected = linear.map(|v| {
                ((if v <= 0.0031308 {
                    v * 12.92
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                }) * 255.0)
                    .round() as u8
            });
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(1.4, 0.16))),
                MeshMaterial3d(standards.add(StandardMaterial {
                    base_color: Color::linear_rgb(
                        linear[0] as f32,
                        linear[1] as f32,
                        linear[2] as f32,
                    ),
                    unlit: true,
                    ..default()
                })),
                Transform::from_xyz(center.x, center.y - 0.77, 0.0),
            ));
            controls.push((center, expected));
        }
    }
    commands.insert_resource(Controls(controls));
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 11.0,
            },
            ..OrthographicProjection::default_3d()
        }),
    ));
    commands.spawn((Text::new("Native triangle SID weights / original x86 basis swatches. Rows: steps1 /2 /4 /8 /16.\nColumns: lower x3 / upper / diagonal / corner / merged / edge / far corner. Same material pipeline."),TextFont{font_size:FontSize::Px(18.0),..default()},
        Node{position_type:PositionType::Absolute,left:px(12.0),top:px(10.0),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, (p, cpu)) in controls.0.iter().enumerate() {
        let gpu = image
            .get_pixel(
                (w as f32 * 0.5 + p.x * h as f32 / 11.0) as u32,
                (h as f32 * 0.5 - p.y * h as f32 / 11.0) as u32,
            )
            .0;
        assert!(
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= 3),
            "triangle{i}: GPU{gpu:?} CPU{cpu:?}"
        );
        info!("Triangle weight{i} PASSED GPU{gpu:?} CPU{cpu:?}");
    }
}
fn world_camera(mut commands: Commands, terrain: Res<aa_world::TerrainQuery>) {
    let target = Vec3::new(
        13825.0,
        10689.0,
        terrain.height_at(13825.0, 10689.0).unwrap(),
    );
    let eye = target + Vec3::new(4.0, -6.0, 5.0);
    let cell = terrain.cell(13, 10).unwrap();
    let s = &cell.surface;
    assert_eq!(
        [
            s[224 * 513 + 256],
            s[224 * 513 + 257],
            s[225 * 513 + 256],
            s[225 * 513 + 257]
        ],
        [12, 1, 1, 5]
    );
    info!("Original triangle-weight border camera Cry{eye:?} target{target:?}, SID[12,1,1,5]");
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(aa_world::cry_to_bevy(eye))
            .looking_at(aa_world::cry_to_bevy(target), Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 55_f32.to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
    ));
}
fn world_check(
    time: Res<Time>,
    mut state: ResMut<State>,
    streamer: Res<aa_world::stream::Streamer>,
    patches: Query<(
        &Mesh3d,
        &MeshMaterial3d<TerrainMaterial>,
        &aa_world::stream::TerrainPatch,
    )>,
    mats: Res<Assets<TerrainMaterial>>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    for (mesh, handle, patch) in &patches {
        if state.meshes.insert(mesh.id()) && !flag("--baseline") {
            let m = meshes.get(mesh.id()).unwrap();
            let Some(VertexAttributeValues::Float32x2(uv1)) = m.attribute(Mesh::ATTRIBUTE_UV_1)
            else {
                panic!("missing patch LOD metadata")
            };
            assert!(
                uv1.iter()
                    .all(|v| *v == [aa_world::terrain::LOD_STEPS[patch.lod] as f32, 0.0])
            );
        }
        if state.mats.insert(handle.id()) {
            state
                .images
                .insert(mats.get(handle.id()).unwrap().extension.surface.id());
        }
    }
    if flag("--unload-check") && !state.moved && time.elapsed_secs() > 12.0 {
        assert!(!state.mats.is_empty());
        state.moved = true;
        camera.single_mut().unwrap().translation =
            aa_world::cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
    }
    if state.moved && !state.done && time.elapsed_secs() > 17.0 {
        assert_eq!(streamer.loaded_count(), 0);
        assert!(!streamer.is_loading());
        assert!(patches.is_empty());
        assert!(state.mats.iter().all(|id| mats.get(*id).is_none()));
        assert!(state.meshes.iter().all(|id| meshes.get(*id).is_none()));
        assert!(state.images.iter().all(|id| images.get(*id).is_none()));
        state.done = true;
        info!(
            "Triangle weights unload PASSED: cells/tasks/patches/materials/surface maps/meshes=0"
        );
    }
}
