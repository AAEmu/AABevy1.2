use super::*;
use bevy::{
    camera::{Viewport, visibility::RenderLayers},
    core_pipeline::{
        prepass::*,
        tonemapping::{DebandDither, Tonemapping},
    },
    render::{extract_component::ExtractComponent, view::NoIndirectDrawing},
};

#[derive(Component, Clone, ExtractComponent)]
#[extract_app(bevy::render::RenderApp)]
pub(crate) struct ProbeView {
    pub z: bool,
    pub number: u32,
}
#[derive(Component, Clone, ExtractComponent)]
#[extract_app(bevy::render::RenderApp)]
pub(crate) struct Source {
    pub group: u32,
    pub fail: u32,
    pub motion_only: bool,
}
#[derive(Component)]
pub(crate) struct Moving;
#[derive(Component)]
pub(crate) struct Backing;
#[derive(Component, Clone, ExtractComponent)]
#[extract_app(bevy::render::RenderApp)]
pub(crate) struct ReadinessStock;

pub(crate) fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ProbeMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut illum: ResMut<Assets<aa_cry::shade::illum::IllumMaterial>>,
) {
    let size = aa_dev::DevArgs::from_env().size;
    let attachment_mode = arg("--attachments").unwrap_or_else(|| "all".into());
    for view in 0..2u32 {
        let mut camera = commands.spawn((
            Camera3d::default(),
            Camera {
                order: view as isize,
                viewport: Some(Viewport {
                    physical_position: UVec2::new(view * size.0 / 2, 0),
                    physical_size: UVec2::new(size.0 / 2, size.1),
                    ..default()
                }),
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::Fixed {
                    width: 8.0,
                    height: 12.0,
                },
                ..OrthographicProjection::default_3d()
            }),
            Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
            Msaa::Off,
            Tonemapping::None,
            DebandDither::Disabled,
            NoBackgroundMotionVectors,
            RenderLayers::layer(view as usize),
            ProbeView {
                z: !flag("--no-z") && view == 0 && attachment_mode != "none",
                number: view,
            },
        ));
        match attachment_mode.as_str() {
            "none" => {}
            "depth" => {
                camera.insert(DepthPrepass);
            }
            "normal" => {
                camera.insert(NormalPrepass);
            }
            "motion" => {
                camera.insert(MotionVectorPrepass);
            }
            _ => {
                camera.insert((DepthPrepass, NormalPrepass, MotionVectorPrepass));
            }
        }
        // Optional diagnostic only, never used as the batching proof.
        if flag("--direct-diagnostic") {
            camera.insert(NoIndirectDrawing);
        }
        if flag("--occlusion") {
            camera.insert(bevy::render::occlusion_culling::OcclusionCulling);
        }
        if view == 1 && flag("--separate-targets") {
            camera.insert(bevy::camera::RenderTarget::Image(
                images
                    .add(Image::new_target_texture(
                        size.0,
                        size.1,
                        TextureFormat::Bgra8UnormSrgb,
                        None,
                    ))
                    .into(),
            ));
        }
        if flag("--unmarked-views") {
            camera.remove::<ProbeView>();
        }
        // COPY_SRC only for attachment verification; no production target changes.
        camera.insert(Camera3d {
            depth_texture_usages: (TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC
                | TextureUsages::TEXTURE_BINDING)
                .into(),
            ..default()
        });
    }
    // Rows: diffuse holes, all holes/high composed, low composed/post-Z,
    // pipeline pair unavailable, resource gate unavailable, frame reset.
    let controls = [
        ([0.0, 1.0, 0.25, 1.0], 0),
        ([0.0, 0.0, 0.25, 1.0], 0),
        ([0.3, 0.8, 0.059, 0.0], 0),
        ([1.0, 1.0, 0.25, 1.0], 1),
        ([1.0, 1.0, 0.25, 1.0], 2),
        ([0.3, 0.8, 0.059, 1.0], 3),
    ];
    let mat = materials.add(ProbeMaterial {
        base: StandardMaterial {
            base_color: Color::linear_rgb(0.0, 1.0, 0.0),
            alpha_mode: AlphaMode::Mask(0.25),
            unlit: true,
            cull_mode: None,
            ..default()
        },
        extension: SuppliedCoverage::default(),
    });
    for (row, (control, fail)) in controls.into_iter().enumerate() {
        if flag("--stock-controls") {
            continue;
        }
        let mut mesh = Mesh::from(Rectangle::new(1.3, 1.4));
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![control; 4]);
        let mesh = meshes.add(mesh);
        for view in 0..2usize {
            for column in 0..3 {
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(-2.4 + column as f32 * 2.4, 4.8 - row as f32 * 1.9, 0.0),
                    RenderLayers::layer(view),
                    Source {
                        group: row as u32,
                        fail,
                        motion_only: false,
                    },
                    Moving,
                ));
            }
        }
    }
    if !flag("--stock-controls") {
        let mesh = meshes.add(Rectangle::new(1.3, 1.4));
        let material = standard.add(StandardMaterial {
            base_color: Color::linear_rgb(0.0, 0.0, 0.25),
            unlit: true,
            ..default()
        });
        for view in 0..2 {
            for column in 0..3 {
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(-2.4 + column as f32 * 2.4, 1.0, -0.3),
                    RenderLayers::layer(view),
                    Backing,
                ));
            }
        }
    }
    // Unmodified stock and actual Illum zero-token controls, kept outside supplied routing.
    let mesh = meshes.add(Rectangle::new(1.2, 0.5));
    for view in 0..2usize {
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(standard.add(StandardMaterial {
                base_color: Color::linear_rgb(0.0, 0.0, 1.0),
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(-1.0, -5.6, 0.0),
            RenderLayers::layer(view),
            ReadinessStock,
        ));
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(illum.add(aa_cry::shade::illum::IllumMaterial {
                base: StandardMaterial {
                    base_color: Color::linear_rgb(1.0, 0.0, 0.0),
                    unlit: true,
                    ..default()
                },
                extension: aa_cry::shade::illum::IllumExt::default(),
            })),
            Transform::from_xyz(1.0, -5.6, 0.0),
            RenderLayers::layer(view),
        ));
        for (index, mode) in [AlphaMode::Blend, AlphaMode::Add].into_iter().enumerate() {
            if flag("--stock-controls") {
                continue;
            }
            let mut supplied = Mesh::from(Rectangle::new(0.8, 0.5));
            supplied.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.3, 0.8, 0.0, 0.5]; 4]);
            let mesh = meshes.add(supplied);
            let material = materials.add(ProbeMaterial {
                base: StandardMaterial {
                    base_color: Color::linear_rgb(0.5, 0.0, 0.5),
                    alpha_mode: mode,
                    unlit: true,
                    cull_mode: None,
                    ..default()
                },
                extension: SuppliedCoverage::default(),
            });
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_xyz(if index == 0 { -3.4 } else { 3.4 }, -5.6, 0.2),
                RenderLayers::layer(view),
                Source {
                    group: 100 + index as u32,
                    fail: 0,
                    motion_only: true,
                },
                Moving,
            ));
        }
        if flag("--shadows") {
            let receiver = standard.add(StandardMaterial {
                base_color: Color::linear_rgb(0.7, 0.7, 0.7),
                perceptual_roughness: 1.0,
                cull_mode: None,
                ..default()
            });
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(6.0, 6.0))),
                MeshMaterial3d(receiver),
                Transform::from_xyz(0.0, 0.0, -0.6),
                RenderLayers::layer(view),
            ));
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.7, 0.7, 0.7))),
                MeshMaterial3d(standard.add(StandardMaterial {
                    base_color: Color::linear_rgb(0.0, 0.0, 1.0),
                    ..default()
                })),
                Transform::from_xyz(-1.0, 0.6, 0.4),
                RenderLayers::layer(view),
            ));
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.7, 0.7, 0.7))),
                MeshMaterial3d(illum.add(aa_cry::shade::illum::IllumMaterial {
                    base: StandardMaterial {
                        base_color: Color::linear_rgb(1.0, 0.0, 0.0),
                        ..default()
                    },
                    extension: aa_cry::shade::illum::IllumExt::default(),
                })),
                Transform::from_xyz(1.0, 0.6, 0.4),
                RenderLayers::layer(view),
            ));
            if flag("--point-light") {
                commands.spawn((
                    PointLight {
                        intensity: 20000.0,
                        range: 20.0,
                        shadow_maps_enabled: !flag("--no-shadows"),
                        ..default()
                    },
                    Transform::from_xyz(2.0, 3.0, 4.0),
                    RenderLayers::layer(view),
                ));
            } else {
                commands.spawn((
                    DirectionalLight {
                        illuminance: 12000.0,
                        shadow_maps_enabled: !flag("--no-shadows"),
                        ..default()
                    },
                    Transform::from_xyz(4.0, 5.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
                    RenderLayers::layer(view),
                ));
            }
        }
    }
}
pub(crate) fn animate(
    mut q: Query<&mut Transform, With<Moving>>,
    mut backing: Query<&mut Transform, (With<Backing>, Without<Moving>)>,
    mut frame: Local<u64>,
) {
    *frame += 1;
    // Deterministic nonzero current/previous world transforms after warmup.
    for mut t in &mut q {
        t.translation.z = if (*frame).is_multiple_of(2) {
            0.05
        } else {
            0.0
        };
        t.translation.x += if (*frame).is_multiple_of(2) {
            0.02
        } else {
            -0.02
        };
    }
    for mut t in &mut backing {
        t.translation.x += if (*frame).is_multiple_of(2) {
            0.04
        } else {
            -0.04
        };
    }
}
pub(crate) fn timings(
    store: Res<bevy::diagnostic::DiagnosticsStore>,
    time: Res<Time>,
    mut last: Local<f32>,
) {
    if time.elapsed_secs() - *last < 2.0 {
        return;
    }
    *last = time.elapsed_secs();
    for d in store.iter() {
        if d.path().as_str().contains("supplied_")
            && let Some(v) = d.average()
        {
            info!(
                "native-pass-probe: timing {}={:.6}{}",
                d.path(),
                v,
                d.suffix
            );
        }
    }
}
