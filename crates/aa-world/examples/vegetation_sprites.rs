//! Isolated original mesh (left) / diagnostic CPU sprite bake (right).
//! The diagnostic also exposes leaf-only models rejected by production atlas admission.
//! --model <pak path> --distance <m> --view <m> --appear <m> --prepass; aa-dev flags apply.
//! Isolation controls: --scale, --yaw/--azimuth/--elevation (degrees), --no-normal-map,
//! --unlit-mesh, --albedo, --atlas-lod0.
use aa_world::objects::{dissolve, sprites};
use bevy::{
    core_pipeline::prepass::{DepthPrepass, NormalPrepass},
    mesh::MeshTag,
    prelude::*,
};
use std::collections::HashMap;

fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args();
    args.find(|a| a == name).and_then(|_| args.next())
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Vegetation mesh / sprite"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            sprites::SpritePlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.12, 0.15, 0.18)))
        .insert_resource(GlobalAmbientLight {
            brightness: 200.0,
            ..default()
        })
        .add_systems(Startup, setup)
        .run();
}

fn setup(
    mut commands: Commands,
    mut cry: aa_cry::CryAssets,
    mut materials: ResMut<Assets<sprites::SpriteMaterial>>,
) {
    let path = arg("--model").unwrap_or_else(|| {
        "objects/env/05_nature/08_cry_source/bushes/cliffbush/cliff_bush_green_mini.cgf".into()
    });
    let options = aa_cry::ModelOptions {
        cry_space: true,
        ..default()
    };
    let model = cry.load_model(&path, &options).expect("original model");
    let loader = cry.loader();
    let material = loader
        .mtl(model.material_path.as_deref().expect("original material"))
        .unwrap();
    let mut textures = HashMap::new();
    let bake = sprites::bake(&model.model, Some(&material), &mut |path| {
        textures.entry(path.to_string()).or_insert_with(|| {
            loader.decode_rgba8(path, arg("--mip").map(|s| s.parse().unwrap()).unwrap_or(2))
                .map(|(w, h, pixels)| std::sync::Arc::new((w as usize, h as usize, pixels)))
        }).clone()
    })
    .expect("nonempty original sprite bake");
    let atlas = bake.upload(&mut cry.images, &mut cry.meshes, &mut materials);
    let albedo_only = std::env::args().any(|a| a == "--albedo");
    if albedo_only {
        materials.get_mut(&atlas.material).unwrap().params.surface.w = 1.0;
    }
    if std::env::args().any(|a| a == "--atlas-lod0") {
        let material = materials.get(&atlas.material).unwrap();
        for handle in [&material.albedo, &material.normal] {
            if let bevy::image::ImageSampler::Descriptor(s) = &mut cry.images.get_mut(handle).unwrap().sampler {
                s.lod_max_clamp = 0.0;
            }
        }
    }
    let distance = arg("--distance")
        .map(|s| s.parse().unwrap())
        .unwrap_or(atlas.radius * 4.5);
    let view = arg("--view").map(|s| s.parse().unwrap()).unwrap_or(2000.0);
    let appear: f32 = arg("--appear").map(|s| s.parse().unwrap()).unwrap_or(0.0);
    let number = |name, default| arg(name).map(|s| s.parse::<f32>().unwrap()).unwrap_or(default);
    let scale = number("--scale", 1.0);
    let yaw = number("--yaw", 0.0).to_radians();
    let azimuth = number("--azimuth", 0.0).to_radians();
    let elevation = number("--elevation", 0.0).to_radians();
    let toward = aa_world::cry_to_bevy(Vec3::new(azimuth.cos(), azimuth.sin(), 0.0));
    let right = (-toward).cross(Vec3::Y);
    let spacing = atlas.half_width * scale * 1.2;
    let rotation = Quat::from_rotation_y(yaw);
    for (node, parts) in &model.parts {
        let transform = Transform::from_matrix(
            Mat4::from_scale_rotation_translation(Vec3::splat(scale), rotation, -right * spacing)
                * Mat4::from_quat(aa_world::coords::CRY_TO_BEVY)
                * Mat4::from(aa_cry::scene::render_transform(&model.model, *node)),
        );
        for part in parts {
            if let Some(mut material) = cry.materials.get_mut(&part.material) {
                if std::env::args().any(|a| a == "--no-normal-map") {
                    material.normal_map_texture = None;
                }
                if albedo_only || std::env::args().any(|a| a == "--unlit-mesh") {
                    material.unlit = true;
                }
            }
            commands.spawn((
                Mesh3d(part.mesh.clone()),
                MeshMaterial3d(part.material.clone()),
                transform,
                part.aabb,
                dissolve::range(if appear > 0.0 { appear.min(view) } else { view }),
            ));
            info!("mesh part bounds {:?}", part.aabb);
        }
    }
    commands.spawn((
        Mesh3d(atlas.mesh.clone()),
        MeshMaterial3d(atlas.material.clone()),
        atlas.aabb,
        Transform { scale: Vec3::splat(scale), rotation, translation: right * spacing },
        dissolve::range(view),
        MeshTag::with_type::<sprites::SpriteMaterial>(sprites::tag(
            appear,
            aa_world::vegetation_color::NO_COLOR_VARIATION,
        )),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 12000.0,
            ..default()
        },
        Transform::from_xyz(3.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let mut camera = commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        Transform::from_translation(toward * (distance * elevation.cos())
            + Vec3::Y * (atlas.center_z * scale + distance * elevation.sin()))
            .looking_at(Vec3::Y * atlas.center_z * scale, Vec3::Y),
        Projection::Perspective(PerspectiveProjection { fov: number("--fov", PerspectiveProjection::default().fov.to_degrees()).to_radians(), ..default() }),
    ));
    if std::env::args().any(|a| a == "--prepass") {
        camera.insert((DepthPrepass, NormalPrepass));
    }
    info!(
        "sprite source {path}: radius={} half_width={} half_height={} centre_z={}",
        atlas.radius, atlas.half_width, atlas.half_height, atlas.center_z
    );
    commands.spawn((Text::new("Original mesh (left) / CPU sprite bake (right)\nIdentical model, material, scale and light; fixed side views, no world post processing"),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        Node { position_type: PositionType::Absolute, top: px(16.0), left: px(16.0), ..default() }));
}
