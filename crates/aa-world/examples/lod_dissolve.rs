//! Physical GPU controls: --grid [--opaque] [--hard-switch], or original rock pair.
//! --fraction 0..1 holds the native camera-distance dissolve; --animate moves out/holds/returns.
use aa_world::objects::{
    dissolve,
    lod_transition::{
        Dissolve, LodDither, LodIllumMaterial, LodStandardMaterial, LodTransitionPlugin,
        MeshLodDither,
    },
};
use bevy::{
    core_pipeline::{
        prepass::{DepthPrepass, NormalPrepass},
        tonemapping::{DebandDither, Tonemapping},
    },
    pbr::ExtendedMaterial,
    prelude::*,
};

fn flag(s: &str) -> bool {
    std::env::args().any(|a| a == s)
}
fn number(s: &str, d: f32) -> f32 {
    let a: Vec<_> = std::env::args().collect();
    a.iter()
        .position(|a| a == s)
        .and_then(|i| a.get(i + 1))
        .map(|a| a.parse().unwrap())
        .unwrap_or(d)
}
#[derive(Component)]
struct Side(bool);
#[derive(Resource)]
struct Motion {
    state: Dissolve,
    previous: f32,
}
fn main() {
    let mut state = Dissolve::new(100.0, 0);
    state.update(100.0, 1);
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native mesh LOD dissolve"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            LodTransitionPlugin,
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(GlobalAmbientLight {
            brightness: 100.0,
            ..default()
        })
        .insert_resource(Motion {
            state,
            previous: -1.0,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, motion)
        .add_systems(Last, counters)
        .run();
}
fn setup(
    mut commands: Commands,
    mut cry: aa_cry::CryAssets,
    mut materials: ResMut<Assets<LodStandardMaterial>>,
    mut illum: ResMut<Assets<LodIllumMaterial>>,
) {
    let grid = flag("--grid");
    if grid {
        let mesh = cry.meshes.add(Rectangle::new(1.5, 1.5));
        for n in 0..=16 {
            let x = (n % 5) as f32 * 2.0 - 4.0;
            let y = 3.0 - (n / 5) as f32 * 2.0;
            let progress = if flag("--hard-switch") {
                if n >= 8 { 16 } else { 0 }
            } else {
                n
            };
            for (secondary, color, level) in [
                (false, Color::srgb(1.0, 0.0, 0.0), progress),
                (true, Color::srgb(0.0, 1.0, 0.0), progress - 16),
            ] {
                let base = StandardMaterial {
                    base_color: color,
                    unlit: true,
                    alpha_mode: if flag("--opaque") {
                        AlphaMode::Opaque
                    } else {
                        AlphaMode::Add
                    },
                    ..default()
                };
                let mut entity = commands.spawn((
                    Mesh3d(mesh.clone()),
                    // Different depths make incorrect prepass dithering observable:
                    // a solid nearer depth would punch holes in the farther colour.
                    Transform::from_xyz(x, y, if secondary { 0.01 } else { 0.0 }),
                    dissolve::range(2000.0),
                    MeshLodDither {
                        level,
                        secondary,
                        active: true,
                    },
                    if (progress == 0 && secondary) || (progress == 16 && !secondary) {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    },
                ));
                if flag("--illum") {
                    entity.insert(MeshMaterial3d(illum.add(ExtendedMaterial {
                        base: ExtendedMaterial {
                            base,
                            extension: aa_cry::shade::illum::IllumExt::default(),
                        },
                        extension: LodDither {},
                    })));
                } else {
                    entity.insert(MeshMaterial3d(materials.add(ExtendedMaterial {
                        base,
                        extension: LodDither {},
                    })));
                }
            }
        }
        commands.spawn((
            Camera3d::default(),
            Transform::from_xyz(0.0, 0.0, 20.0),
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                    viewport_height: 10.0,
                },
                ..OrthographicProjection::default_3d()
            }),
            Tonemapping::None,
            DebandDither::Disabled,
            Msaa::Off,
            DepthPrepass,
            NormalPrepass,
        ));
        info!(
            "LOD GPU grid: 17 paired tiles, 34 prepared mesh parts, 68 triangles, additive={} hard_switch={}",
            !flag("--opaque"),
            flag("--hard-switch")
        );
        return;
    }
    let data = aa_world::placement::load_cell_objects("main_world", 9, 12);
    let b = &data.brushes[427];
    let path = &data.models[b.model as usize];
    assert!(path.ends_with("/cliff_rock_a_small.cgf"));
    let center = (Vec3::from_slice(&b.bbox[..3]) + Vec3::from_slice(&b.bbox[3..])) * 0.5;
    let rotation = Mat4::from_quat(aa_cry::CRY_TO_BEVY);
    let mut total = 0;
    for level in 0..=1 {
        let model = cry
            .load_model(
                path,
                &aa_cry::ModelOptions {
                    lod: level,
                    cry_space: true,
                    keep_cpu_mesh: true,
                    ..default()
                },
            )
            .unwrap();
        let mut triangles = 0;
        for (node, parts) in &model.parts {
            let transform = Transform::from_matrix(
                Mat4::from_translation(-aa_world::cry_to_bevy(center))
                    * rotation
                    * b.mat4()
                    * node_world(&model.model, *node),
            );
            for part in parts {
                triangles += cry.meshes.get(&part.mesh).unwrap().indices().unwrap().len() / 3;
                let base = cry.materials.get(&part.material).unwrap().clone();
                let material = materials.add(ExtendedMaterial {
                    base,
                    extension: LodDither {},
                });
                commands.spawn((
                    Mesh3d(part.mesh.clone()),
                    MeshMaterial3d(material),
                    transform,
                    part.aabb,
                    dissolve::range(2000.0),
                    MeshLodDither::default(),
                    Side(level == 1),
                ));
            }
        }
        info!(
            "original rock LOD{level}: nodes={} triangles={triangles}",
            model.parts.len()
        );
        total += triangles;
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 12000.0,
            ..default()
        },
        Transform::from_xyz(-3.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(25.0, 16.0, 32.0).looking_at(Vec3::ZERO, Vec3::Y),
        DepthPrepass,
        NormalPrepass,
    ));
    info!(
        "original paired rock control: {total} triangles during dissolve, 320/168 at endpoints; fixed matched camera/material/light"
    );
}
fn node_world(model: &aa_cry::Model, index: usize) -> Mat4 {
    let node = &model.nodes[index];
    let local = if matches!(node.kind, aa_cry::mesh::NodeKind::Lod(_)) {
        Mat4::IDENTITY
    } else {
        Mat4::from(node.local)
    };
    if let Some(parent) = node.parent.filter(|p| *p != index) {
        node_world(model, parent) * local
    } else {
        local
    }
}
#[allow(clippy::type_complexity)]
fn counters(
    time: Res<Time>,
    parts: Query<(
        &ViewVisibility,
        &Mesh3d,
        Option<&MeshMaterial3d<LodStandardMaterial>>,
        Option<&MeshMaterial3d<LodIllumMaterial>>,
    )>,
    mut next: Local<f32>,
) {
    if time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 2.0;
    let prepared: Vec<_> = parts
        .iter()
        .filter(|(_, _, a, b)| a.is_some() || b.is_some())
        .collect();
    let visible: Vec<_> = prepared.iter().filter(|(v, _, _, _)| v.get()).collect();
    let groups: std::collections::HashSet<_> = visible
        .iter()
        .map(|(_, m, a, b)| {
            (
                m.id(),
                a.map(|h| h.id().untyped())
                    .unwrap_or_else(|| b.unwrap().id().untyped()),
            )
        })
        .collect();
    info!(
        "LOD GPU prepared_parts={} visible_parts={} mesh_material_draw_groups={}",
        prepared.len(),
        visible.len(),
        groups.len()
    );
}
fn motion(
    time: Res<Time>,
    mut motion: ResMut<Motion>,
    mut sides: Query<(&Side, &mut MeshLodDither, &mut Visibility)>,
) {
    let fraction = if flag("--animate") {
        let t = time.elapsed_secs() % 12.0;
        if t < 3.0 {
            (t / 3.0).min(0.5)
        } else if t < 6.0 {
            0.5
        } else if t < 9.0 {
            1.0
        } else {
            0.0
        }
    } else {
        number("--fraction", 0.5)
    };
    let mut n = (fraction * 16.0).round() as i32;
    if flag("--hard-switch") {
        n = if fraction >= 0.5 { 16 } else { 0 };
    }
    if motion.previous != fraction {
        let reference = motion.state.update(100.0 + fraction * 5.0, 1);
        info!(
            "LOD control camera-distance={} native_reference={reference} dither_step={n} hard_switch={}",
            100.0 + fraction * 5.0,
            flag("--hard-switch")
        );
        motion.previous = fraction;
    }
    for (side, mut d, mut visibility) in &mut sides {
        *d = MeshLodDither {
            level: if side.0 { n - 16 } else { n },
            secondary: side.0,
            active: true,
        };
        *visibility = if (n == 0 && side.0) || (n == 16 && !side.0) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
}
