//! Controlled Illum forward-light fixture. Uses authored materials with `--material palace|armour`.
//! `--kind none|point|spot|directional|mixed`, `--term all|diffuse|highlight|reflection`,
//! `--distance N`, `--range N`, `--light-x N`, `--aim-x N`, `--cookie white|black|checker`,
//! `--shadows`, `--count N`. The default plane has a constant normal and an orthographic view.

use aa_cry::shade::illum::{IllumExt, IllumLightSpecular, IllumMaterial, IllumParams};
use aa_cry::{CryAssets, CryPlugin, ModelOptions, Shading};
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::light::SpotLightTexture;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

fn arg(name: &str, default: &str) -> String {
    let args: Vec<_> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map_or_else(|| default.into(), Clone::clone)
}

fn number(name: &str, default: f32) -> f32 {
    arg(name, "").parse().unwrap_or(default)
}

fn main() {
    let mut plugins = aa_dev::default_plugins("aa-cry: Illum local lights").build();
    let no_bindless = std::env::args().any(|a| a == "--no-bindless");
    if no_bindless || std::env::args().any(|a| a == "--no-buffer-arrays") {
        plugins = plugins.set(bevy::render::RenderPlugin {
            render_creation: bevy::render::settings::WgpuSettings {
                disabled_features: Some(
                    bevy::render::settings::WgpuFeatures::BUFFER_BINDING_ARRAY
                        | if no_bindless {
                            bevy::render::settings::WgpuFeatures::TEXTURE_BINDING_ARRAY
                        } else {
                            bevy::render::settings::WgpuFeatures::empty()
                        },
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
        .add_systems(Startup, (setup, setup_churn).chain())
        .add_systems(Update, (isolate_term, churn));
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.add_systems(bevy::render::RenderStartup, |device: Res<bevy::render::renderer::RenderDevice>| {
            info!("illum_lights: material_bindless={}", <IllumMaterial as bevy::render::render_resource::AsBindGroup>::bindless_supported(&device));
        });
    }
    app.run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, mut native: ResMut<Assets<IllumMaterial>>) {
    let material = arg("--material", "fixture");
    if material == "fixture" {
        let source = aa_cry::material::Material {
            shader: "Illum".into(),
            diffuse: Vec3::splat(0.25),
            specular: Vec3::splat(0.7),
            shininess: 16.0,
            ..default()
        };
        let mut params = IllumParams::from_material(&source, false, false);
        params.vertex.z = 0.0;
        commands.spawn((
            Mesh3d(cry.meshes.add(Rectangle::new(6.0, 4.0))),
            MeshMaterial3d(native.add(IllumMaterial {
                base: StandardMaterial {
                    base_color: Color::linear_rgb(0.25, 0.25, 0.25),
                    reflectance: 0.0,
                    perceptual_roughness: 1.0,
                    ..default()
                },
                extension: IllumExt {
                    params,
                    ..default()
                },
            })),
            Transform::default(),
        ));
    } else {
        let (path, override_mtl) = match material.as_str() {
            "palace" => (
                "objects/env/01_nuia/01_sol/castle_nuia_001.cgf",
                Some("objects/env/01_nuia/01_sol/castle_nuia_001_twocrowns.mtl".into()),
            ),
            "armour" => (
                "objects/characters/nuian/female/costume/nu_f_sk_saint001/nu_f_sk_saint001.chr",
                None,
            ),
            _ => panic!("unknown --material {material}"),
        };
        let options = ModelOptions {
            shading: Shading::Full,
            material: override_mtl,
            ..default()
        };
        let loaded = cry.load_model(path, &options).expect("original model");
        let (lo, hi) = loaded.model.bounds().expect("bounds");
        let center = aa_cry::CRY_TO_BEVY * ((lo + hi) * 0.5);
        let scale = 3.4 / (hi - lo).max_element();
        let rotation = Quat::from_rotation_y(std::f32::consts::PI);
        let root = cry
            .spawn_model_full(&mut commands, path, &options)
            .expect("original materials")
            .root;
        commands.entity(root).insert(Transform {
            translation: -(rotation * center) * scale,
            rotation: rotation * aa_cry::CRY_TO_BEVY,
            scale: Vec3::splat(scale),
        });
        info!(
            "illum_lights: original={path}; material={:?}; display scale={scale}",
            loaded.material_path
        );
    }

    let kind = arg("--kind", "point");
    let count = number("--count", 1.0).clamp(1.0, 128.0) as usize;
    let distance = number("--distance", 2.0);
    let range = number("--range", 5.0);
    let light_x = number("--light-x", 0.7);
    let aim_x = number("--aim-x", 0.0);
    let shadows = std::env::args().any(|a| a == "--shadows");
    // Same half-radius conversion as aa-world's projectors. One fixture source unit is 0.35 stage units.
    let amplitude = aa_cry::stage::LIGHT_UNIT_LUX * 0.35;
    let intensity = 4.0 * std::f32::consts::PI * (16.0 / 75.0) * range * range * amplitude;
    let cookie = arg("--cookie", "white");
    let specular = arg("--specular", "").parse::<f32>().ok();
    let cookie_image = if cookie != "white" {
        let rgba: Vec<_> = (0..16)
            .flat_map(|i| {
                let white = cookie == "checker" && (i % 4 + i / 4) % 2 == 0;
                let c = if white { 255 } else { 0 };
                [c, c, c, 255]
            })
            .collect();
        Some(cry.images.add(Image::new(
            Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        )))
    } else {
        None
    };
    let mut point_count = 0;
    let mut spot_count = 0;
    for i in 0..count {
        let position = Vec3::new(
            light_x + (i % 4) as f32 * 0.12,
            0.5 + (i / 4) as f32 * 0.04,
            distance,
        );
        if kind == "point" || kind == "mixed" {
            let mut entity = commands.spawn((
                PointLight {
                    intensity,
                    range,
                    radius: 0.0,
                    shadow_maps_enabled: shadows,
                    ..default()
                },
                Transform::from_translation(position),
            ));
            if let Some(value) = specular {
                entity.insert(IllumLightSpecular(value));
            }
            point_count += 1;
        }
        if kind == "spot" || kind == "mixed" {
            let mut entity = commands.spawn((
                SpotLight {
                    intensity,
                    range,
                    radius: 0.0,
                    inner_angle: 0.3,
                    outer_angle: 0.65,
                    shadow_maps_enabled: shadows,
                    ..default()
                },
                Transform::from_translation(position)
                    .looking_at(Vec3::new(aim_x, 0.0, 0.0), Vec3::Y),
            ));
            if let Some(image) = &cookie_image {
                entity.insert(SpotLightTexture {
                    image: image.clone(),
                });
            }
            if let Some(value) = specular {
                entity.insert(IllumLightSpecular(value));
            }
            spot_count += 1;
        }
    }
    if kind == "directional" || kind == "mixed" {
        commands.spawn((
            DirectionalLight {
                illuminance: amplitude,
                shadow_maps_enabled: true,
                ..default()
            },
            Transform::from_xyz(-0.7, 0.5, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
    }
    if shadows {
        commands.spawn((
            Mesh3d(cry.meshes.add(Cuboid::new(0.45, 1.6, 0.12))),
            MeshMaterial3d(cry.materials.add(StandardMaterial {
                base_color: Color::BLACK,
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(-0.2, 0.0, 0.8),
        ));
    }
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 4.5,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    info!(
        "illum_lights: material={material}; kind={kind}; term={}; point={point_count}; spot={spot_count}; directional={}; distance={distance}; range={range}; x={light_x}; aim_x={aim_x}; cookie={cookie}; shadows={shadows}; specular={specular:?}; lumens={intensity}; amplitude_lux={amplitude}",
        arg("--term", "all"),
        usize::from(kind == "directional" || kind == "mixed")
    );
}

#[derive(Resource)]
struct Churn {
    green: Entity,
    red: Entity,
    blue: Entity,
    phase: u8,
}

fn coloured_light(commands: &mut Commands, colour: Color, specular: f32, spot: bool) -> Entity {
    let range = 5.0;
    let intensity = 4.0
        * std::f32::consts::PI
        * (16.0 / 75.0)
        * range
        * range
        * aa_cry::stage::LIGHT_UNIT_LUX
        * 0.35;
    let mut entity = commands.spawn((
        IllumLightSpecular(specular),
        Transform::from_xyz(0.7, 0.5, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    if spot {
        entity.insert(SpotLight {
            color: colour,
            intensity,
            range,
            radius: 0.0,
            inner_angle: 0.3,
            outer_angle: 0.65,
            shadow_maps_enabled: true,
            ..default()
        });
    } else {
        entity.insert(PointLight {
            color: colour,
            intensity,
            range,
            radius: 0.0,
            ..default()
        });
    }
    entity.id()
}

fn setup_churn(mut commands: Commands) {
    let live = std::env::args().any(|a| a == "--churn");
    let final_state = std::env::args().any(|a| a == "--churn-final");
    if !live && !final_state {
        return;
    }
    let red = coloured_light(
        &mut commands,
        Color::linear_rgb(1.0, 0.0, 0.0),
        if live { 0.0 } else { 1.0 / 3.0 },
        false,
    );
    let green = coloured_light(
        &mut commands,
        Color::linear_rgb(0.0, 1.0, 0.0),
        if live { 0.5 } else { 0.02 },
        false,
    );
    let blue = coloured_light(
        &mut commands,
        Color::linear_rgb(0.0, 0.0, 1.0),
        if live { 1.0 / 3.0 } else { 0.5 },
        true,
    );
    if live {
        commands.insert_resource(Churn {
            red,
            green,
            blue,
            phase: 0,
        });
    }
    info!("illum_lights: churn={live} final={final_state}; point=2; spot=1; directional=0");
}

fn churn(mut commands: Commands, time: Res<Time>, state: Option<ResMut<Churn>>) {
    let Some(mut state) = state else { return };
    if state.phase == 0 && time.elapsed_secs() >= 2.0 {
        commands.entity(state.red).despawn();
        commands.entity(state.green).remove::<IllumLightSpecular>();
        commands.entity(state.blue).insert(IllumLightSpecular(0.0));
        state.red = coloured_light(&mut commands, Color::linear_rgb(1.0, 0.0, 0.0), 0.5, false);
        state.phase = 1;
        info!(
            "illum_lights: churn phase1: despawn/reindex, remove component -> 1, explicit zero; point=2 spot=1"
        );
    } else if state.phase == 1 && time.elapsed_secs() >= 4.0 {
        commands.entity(state.red).despawn();
        commands.entity(state.blue).despawn();
        commands
            .entity(state.green)
            .insert(IllumLightSpecular(0.02));
        state.red = coloured_light(
            &mut commands,
            Color::linear_rgb(1.0, 0.0, 0.0),
            1.0 / 3.0,
            false,
        );
        state.blue = coloured_light(&mut commands, Color::linear_rgb(0.0, 0.0, 1.0), 0.5, true);
        state.phase = 2;
        info!(
            "illum_lights: churn phase2: respawn/remap; final RGB spec=1/3,.02,.5; point=2 spot=1"
        );
    }
}

// Also reaches the automatic material swaps for the original meshes. Does not modify their authored parameters.
fn isolate_term(
    mut materials: ResMut<Assets<IllumMaterial>>,
    mut seen: Local<std::collections::HashSet<AssetId<IllumMaterial>>>,
) {
    let term = arg("--term", "all");
    if term == "all" {
        return;
    }
    for (id, material) in materials.iter_mut() {
        if !seen.insert(id) {
            continue;
        }
        match term.as_str() {
            "highlight" => material.extension.params.gain.z = 2.0,
            "reflection" => material.extension.params.gain.z = 1.0,
            "diffuse" => {
                material.extension.params.spec = Vec4::ZERO;
                material.extension.params.env.x = 0.0;
            }
            _ => panic!("unknown --term {term}"),
        }
    }
}
