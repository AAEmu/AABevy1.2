//! Original Illum alpha/glow assets. `--asset bow|road|hammer`, `--quad`, `--unlit`,
//! `--full`, `--token-zero`, `--no-colors`, `--no-bindless`, `--glow-only`, `--prepass`.
use aa_cry::{CryAssets, CryPlugin, ModelOptions};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() {
    let mut plugins = aa_dev::default_plugins("aa-cry: original Illum alpha/glow").build();
    if flag("--no-bindless") {
        plugins = plugins.set(bevy::render::RenderPlugin {
            render_creation: bevy::render::settings::WgpuSettings {
                disabled_features: Some(
                    bevy::render::settings::WgpuFeatures::TEXTURE_BINDING_ARRAY,
                ),
                ..default()
            }
            .into(),
            ..default()
        });
    }
    let mut app = App::new();
    app.add_plugins((plugins, aa_dev::DevPlugin, CryPlugin))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(GlobalAmbientLight {
            brightness: 0.0,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Last, controls);
    // Deterministic render time; main-world aa-dev screenshot/FPS time continues.
    app.sub_app_mut(RenderApp).add_systems(
        Render,
        (|mut time: ResMut<Time>| {
            *time = Time::default();
            time.advance_by(std::time::Duration::from_secs(2));
        })
        .in_set(RenderSystems::PrepareAssets),
    );
    app.run();
}

fn setup(mut commands: Commands, mut cry: CryAssets) {
    let asset = arg("--asset").unwrap_or_else(|| "bow".into());
    let (model, material, sub) = match asset.as_str() {
        "road" => (
            "objects/item/bow/bow_ean_1/bow_ean_1.cgf",
            "materials/aa_terrain/roads/road_field_dirt_201a.mtl",
            0,
        ),
        "hammer" => (
            "objects/effects/haje_hammer.cgf",
            "objects/effects/haje_hammer_second1.mtl",
            0,
        ),
        "plant" => (
            "objects/env/03_origin_continent/001_odeco/odeco_plants_d01.cgf",
            "objects/env/03_origin_continent/001_odeco/odeco_plants_d1.mtl",
            1,
        ),
        "coral" => (
            "objects/env/05_nature/00_common/waterplants/common_waterbig_d02.cgf",
            "objects/env/05_nature/00_common/waterplants/common_waterbig_d02.mtl",
            1,
        ),
        _ => (
            "objects/item/bow/bow_ean_1/bow_ean_1.cgf",
            "objects/item/bow/bow_ean_1/bow_ean_1.mtl",
            0,
        ),
    };
    let options = ModelOptions {
        material: Some(material.into()),
        shading: if flag("--full") {
            aa_cry::scene::Shading::Full
        } else {
            aa_cry::scene::Shading::Basic
        },
        material_hook: Some(aa_cry::MaterialHook::new(|_, m| {
            if flag("--unlit") {
                m.unlit = true;
            }
            if flag("--glow-only") {
                m.base_color = Color::BLACK;
                m.emissive = LinearRgba::BLACK;
                m.reflectance = 0.0;
            }
        })),
        ..default()
    };
    let loaded = cry.load_model(model, &options).unwrap();
    let (center, distance) = if flag("--quad") {
        let source = aa_cry::material::Material::load(material).unwrap();
        let source = source.sub(sub).unwrap_or(&source);
        let diffuse = cry.texture(
            &source
                .texture(&aa_cry::material::TextureMap::Diffuse)
                .unwrap()
                .pak_path(),
            true,
        );
        let handle = loaded
            .parts
            .iter()
            .flat_map(|(_, parts)| parts)
            .find_map(|part| {
                (cry.materials
                    .get(&part.material)
                    .unwrap()
                    .base_color_texture
                    == diffuse)
                    .then(|| part.material.clone())
            })
            .expect("original material subset");
        let mut m = cry.materials.get_mut(&handle).unwrap();
        if flag("--unlit") {
            m.unlit = true;
        }
        if flag("--glow-only") {
            m.base_color = Color::BLACK;
            m.emissive = LinearRgba::BLACK;
            m.reflectance = 0.0;
        }
        // Controlled vertex gradient isolates interpolation and native flag precedence.
        let mut mesh = Mesh::from(Rectangle::new(2.0, 2.0));
        if let Some(window) = arg("--uv-window").and_then(|s| s.parse::<f32>().ok())
            && let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) =
                mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0)
        {
            for uv in uvs {
                for v in uv {
                    *v = (*v - 0.5) * window + 0.5;
                }
            }
        }
        if !flag("--no-colors") {
            let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
                bevy::mesh::VertexAttributeValues::Float32x3(p) => p,
                _ => unreachable!(),
            };
            let colors: Vec<_> = positions
                .iter()
                .map(|p| {
                    let a = (p[0] + 1.0) * 0.5;
                    [0.25, 0.5, 0.75, a]
                })
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
        info!(
            "illum_alpha: source={material} mask={:x} diffuse={:?} opacity={} params={:?} uv={:?}",
            source.gen_mask, source.diffuse, source.opacity, source.public_params, m.uv_transform
        );
        let mut entity = commands.spawn((
            Mesh3d(cry.meshes.add(mesh)),
            MeshMaterial3d(handle),
            Transform::default(),
        ));
        if flag("--dissolve") {
            entity.insert(bevy::camera::visibility::VisibilityRange {
                start_margin: 0.0..0.0,
                end_margin: 2.0..4.0,
                use_aabb: false,
            });
        }
        (Vec3::ZERO, 3.0)
    } else {
        for mesh in &loaded.model.meshes {
            let (min, max) = mesh
                .colors
                .iter()
                .map(|c| c[3])
                .fold((255, 0), |(min, max), a| (min.min(a), max.max(a)));
            info!(
                "illum_alpha: original mesh colors={} alpha={min}..{max}",
                mesh.colors.len()
            );
        }
        aa_cry::scene::spawn_loaded(&mut commands, model, &loaded, &options);
        let (a, b) = loaded.model.bounds().unwrap();
        let lo = Vec3::new(a.x, a.z, -b.y);
        let hi = Vec3::new(b.x, b.z, -a.y);
        ((lo + hi) * 0.5, (hi - lo).length() * 1.15)
    };
    let mut camera = commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: if flag("--quad") { 3.0 } else { distance },
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(center + Quat::from_rotation_y(
            arg("--yaw").and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0).to_radians()
        ) * Vec3::Z * distance).looking_at(center, Vec3::Y),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        // Existing Illum engine-unit bridge: 998 * view.exposure = 1.
        bevy::camera::Exposure {
            ev100: (998.0_f32 / 1.2).log2(),
        },
    ));
    if flag("--prepass") {
        camera.insert((
            bevy::core_pipeline::prepass::DepthPrepass,
            bevy::core_pipeline::prepass::NormalPrepass,
            bevy::core_pipeline::prepass::MotionVectorPrepass,
        ));
    }
    if !flag("--glow-only") && !flag("--unlit") {
        commands.spawn((
            DirectionalLight {
                illuminance: 2500.0,
                shadow_maps_enabled: flag("--shadows"),
                ..default()
            },
            Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -0.5, -0.6, 0.0)),
        ));
    }
    info!(
        "illum_alpha: asset={asset} quad={} full={} fixed render time=2",
        flag("--quad"),
        flag("--full")
    );
}

fn controls(mut materials: ResMut<Assets<aa_cry::shade::illum::IllumMaterial>>) {
    for (_, material) in materials.iter_mut() {
        if flag("--token-zero") {
            material.extension.params.vertex.w = 0.0;
        }
        if flag("--glow-only") {
            material.extension.params.emission = Vec4::ZERO;
            material.extension.params.spec = Vec4::ZERO;
            material.extension.params.env.x = 0.0;
        }
    }
}
