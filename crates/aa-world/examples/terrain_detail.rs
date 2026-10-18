//! Native ATI2 detail channels: raw-texel GPU controls, or --world [--forest] [--off] [--unload-check].
//! --off restores the previous detail component order; base normals/geometry/lighting are identical.
use aa_cry::texture::{Dds, DdsOptions, dds_to_image};
use aa_world::terrain_layers::{
    LayerLibrary, TerrainExt, TerrainMaterial, TerrainMaterialPlugin, TerrainParams,
};
use bevy::{
    asset::AssetId,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    image::ImageSampler,
    prelude::*,
    render::{
        render_resource::{TextureViewDescriptor, TextureViewDimension},
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
struct Scene {
    entities: HashSet<Entity>,
    materials: HashSet<AssetId<TerrainMaterial>>,
    changed: usize,
    moved: bool,
    done: bool,
    last: f32,
}
fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native heightmap detail channels"),
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
                hour: 10.,
                ..default()
            })
            .init_resource::<Scene>()
            .add_systems(Startup, world_setup)
            .add_systems(Update, world_check);
    } else {
        app.add_plugins(TerrainMaterialPlugin)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup, numeric)
            .add_observer(pixels);
    }
    app.run();
}
fn raw(path: &str, point: UVec2, prepared_array: bool) -> (Vec3, Vec2, Image) {
    let bytes = aa_cry::read(path).unwrap();
    let mut dds = Dds::parse(&bytes).unwrap();
    dds.swapped_rg = false;
    let mip = if prepared_array && dds.width > 512 {
        (dds.width / 512).trailing_zeros()
    } else {
        0
    };
    let width = dds.width >> mip;
    let height = dds.height >> mip;
    let i = ((point.y * width + point.x) * 4) as usize;
    let pixel = dds.decode_rgba8(0, mip);
    let rgb = Vec3::new(pixel[i] as f32, pixel[i + 1] as f32, pixel[i + 2] as f32) / 255.;
    let uv = (point.as_vec2() + Vec2::splat(0.5)) / Vec2::new(width as f32, height as f32);
    info!("Original terrain raw texel {path} mip{mip} point{point:?}: {rgb:?}");
    (rgb, uv, dds_to_image(&dds, DdsOptions::linear()).unwrap())
}
fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    let built = LayerLibrary::build("main_world", [(20, 8)].into_iter()).unwrap();
    let original = built.library.params((20, 8), Vec3::ONE).unwrap();
    let source = 12 * 3 * 4; // Original rock406b X projection, normal + detail scale1.
    let (base, base_uv, _) = raw(
        "textures/aa_terrain/canyon/tr_canyon_rock3d_406b01_ddn.dds",
        UVec2::ZERO,
        true,
    );
    let (detail, detail_uv, _) = raw(
        "textures/aa_terrain/canyon/cliff_grey_blue_with_dry_bushes_ddn.dds",
        UVec2::ZERO,
        true,
    );
    let (bc1, bc1_uv, mut bc1_image) = raw(
        "textures/aa_terrain/field/tr_field_dirt_001a_ddn.dds",
        UVec2::new(10, 311),
        false,
    );
    bc1_image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    bc1_image.sampler = ImageSampler::nearest();
    let bc1_handle = images.add(bc1_image);
    let arrays = built.images.map(|mut image| {
        image.sampler = ImageSampler::nearest();
        images.add(image)
    });
    let surface = images.add(aa_world::terrain_layers::surface_image(&[0]));
    let mut controls = Vec::new();
    for row in 0..3 {
        for (column, strength) in [0., 0.2, 1., -0.7].into_iter().enumerate() {
            let center = Vec2::new(column as f32 * 2.0 - 3.0, 2.1 - row as f32 * 2.1);
            let mut params = TerrainParams::new(Vec3::ONE);
            params.inspect_normal = 1;
            params.tint.w = 1e9;
            let entry = 2 * 4; // Controlled flat +Z Cry receiver, weight1.
            params.layers[entry..entry + 4].copy_from_slice(&original.layers[source..source + 4]);
            params.layers[entry].z = 0.;
            params.layers[entry].w = 0.;
            params.layers[entry + 1].x = base_uv.x;
            params.layers[entry + 1].y = base_uv.y;
            params.layers[entry + 1].w = strength;
            params.layers[entry + 2].y = 0.;
            params.layers[entry + 2].z = 0.;
            let mut bindings = arrays.clone();
            let mut duv = detail_uv;
            if row == 1 {
                // Exercise second-array + copied-source flags together, with the
                // same actual array/texel, so only metadata routing differs.
                let flags = (params.layers[entry + 2].x / 1000.).floor() as u32;
                assert_eq!(flags & 6, 4);
                params.layers[entry + 2].x += 2000.;
                if flags & 1 == 0 {
                    params.layers[entry + 2].x += 1000.;
                    duv *= 2.;
                }
                bindings[4] = arrays[1].clone();
            } else if row == 2 {
                // Native BC1 detail reads its plain RG; ATI2 provenance must not
                // affect another format sharing the same sampler/second-array path.
                params.layers[entry + 2].x = 2000.;
                duv = bc1_uv;
                bindings[4] = bc1_handle.clone();
            }
            params.layers[entry + 3].x = duv.x;
            params.layers[entry + 3].y = duv.y;
            let mut mesh = Mesh::from(Rectangle::new(1.6, 1.35));
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; 4]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0., 0.]; 4]);
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(TerrainMaterial {
                    base: StandardMaterial {
                        unlit: true,
                        ..default()
                    },
                    extension: TerrainExt::new(params, bindings, surface.clone()),
                })),
                Transform::from_xyz(center.x, center.y, 0.),
            ));
            // Source Common.cfi: base raw YX, expand, reconstruct Z, then
            // FragLib.cfi: add detail raw XY*scale, keeping reconstructed base Z.
            let xy = Vec2::new(base.y, base.x) * 2. - Vec2::ONE;
            let d = if row == 2 { bc1 } else { detail };
            let bump = Vec3::new(
                xy.x + (d.x * 2. - 1.) * strength,
                xy.y + (d.y * 2. - 1.) * strength,
                (1. - xy.length_squared()).max(0.).sqrt(),
            );
            // Existing flat heightmap frame: U=+X,V=-Z,N=+Y in Bevy space.
            let n = Vec3::new(bump.x, bump.z, -bump.y).normalize();
            let linear = n * 0.5 + Vec3::splat(0.5);
            let rgb = linear.to_array().map(|v| {
                ((if v <= 0.0031308 {
                    v * 12.92
                } else {
                    1.055 * v.powf(1. / 2.4) - 0.055
                }) * 255.)
                    .round() as u8
            });
            controls.push((center, rgb));
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(1.6, 0.2))),
                MeshMaterial3d(standards.add(StandardMaterial {
                    base_color: Color::linear_rgb(linear.x, linear.y, linear.z),
                    unlit: true,
                    ..default()
                })),
                Transform::from_xyz(center.x, center.y - 0.82, 0.),
            ));
        }
    }
    commands.insert_resource(Controls(controls));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0., 0., 6.).looking_at(Vec3::ZERO, Vec3::Y),
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
    commands.spawn((Text::new("Native heightmap detail normals; actual rock/ATI2/BC1 texels, independent CPU swatches\nRows: native ATI2 / second array+copy flags / plain BC1 detail\nColumns: detail scale 0 / .2 / 1 / -.7. Reconstruct base Z before adding detail XY."),TextFont{font_size:FontSize::Px(18.),..default()},Node{position_type:PositionType::Absolute,left:px(15.),top:px(10.),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, (point, cpu)) in controls.0.iter().enumerate() {
        let x = (w as f32 * 0.5 + point.x * h as f32 / 8.) as u32;
        let y = (h as f32 * 0.5 - point.y * h as f32 / 8.) as u32;
        let gpu = image.get_pixel(x, y).0;
        assert!(
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= 2),
            "terrain detail{i} GPU{gpu:?} CPU{cpu:?}"
        );
        info!("Native terrain detail GPU case{i} PASSED GPU{gpu:?} independentCPU{cpu:?}");
    }
}
fn world_setup(mut commands: Commands) {
    let (target, offset) = if flag("--forest") {
        (
            Vec3::new(12812.47, 14575.70, 158.27849),
            Vec3::new(30., -40., 14.),
        )
    } else {
        (
            Vec3::new(21075., 8552.923, 374.),
            Vec3::new(16.446, -24., 10.),
        )
    };
    let eye = target + offset;
    info!("Original terrain detail camera Cry{eye:?} target{target:?}");
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(aa_world::cry_to_bevy(eye))
            .looking_at(aa_world::cry_to_bevy(target), Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 50_f32.to_radians(),
            near: 0.1,
            far: 20000.,
            ..default()
        }),
    ));
}
fn world_check(
    time: Res<Time>,
    stream: Res<aa_world::stream::Streamer>,
    patches: Query<(
        Entity,
        &aa_world::stream::TerrainPatch,
        &MeshMaterial3d<TerrainMaterial>,
    )>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    all: Query<()>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mut scene: ResMut<Scene>,
) {
    for (entity, _, material) in &patches {
        scene.entities.insert(entity);
        if scene.materials.insert(material.0.id()) {
            let mut material = materials.get_mut(&material.0).unwrap();
            for entry in material.extension.params.layers.chunks_mut(4) {
                let code = entry[2].x;
                if code >= 0. && ((code / 1000.).floor() as u32) & 4 != 0 {
                    scene.changed += 1;
                    if flag("--off") {
                        entry[2].x -= 4000.;
                    }
                }
            }
        }
    }
    let now = time.elapsed_secs();
    if now - scene.last > 2. {
        scene.last = now;
        info!(
            "terrain detail cells={} patches={} loading={} materials_seen={} ATI2_detail_refs={} legacy={}",
            stream.loaded_count(),
            patches.iter().count(),
            stream.is_loading(),
            scene.materials.len(),
            scene.changed,
            flag("--off")
        );
    }
    if flag("--unload-check") && !scene.moved && now > 16. {
        let expected = if flag("--forest") { (12, 14) } else { (20, 8) };
        assert!(patches.iter().any(|(_, p, _)| p.cell == expected));
        assert!(scene.changed > 0);
        assert!(!stream.is_loading());
        cameras.single_mut().unwrap().translation = Vec3::splat(-100000.);
        scene.moved = true;
    }
    if scene.moved && !scene.done && now > 23. {
        assert!(patches.is_empty());
        assert_eq!(stream.loaded_count(), 0);
        assert!(!stream.is_loading());
        assert!(scene.entities.iter().all(|e| all.get(*e).is_err()));
        assert!(scene.materials.iter().all(|m| !materials.contains(*m)));
        scene.done = true;
        info!(
            "Native terrain detail unload PASSED: {} patch entities and {} cell materials gone; shared world arrays unchanged",
            scene.entities.len(),
            scene.materials.len()
        );
    }
}
