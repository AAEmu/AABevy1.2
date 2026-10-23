//! Original waterfall mesh/material, with fixed render-time controls for matched captures.
//! `--asset size15|ma|second`, `--quad` (original maps on a diagnostic UV square),
//! `--time seconds` (otherwise live), `--unlit`, `--token-zero`, `--no-bindless`.
use aa_cry::{CryAssets, CryPlugin, ModelOptions};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};

fn arg(flag: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn main() {
    let mut plugins = aa_dev::default_plugins("aa-cry: original waterfall motion").build();
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
            brightness: 900.0,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Last, token_zero);
    if let Some(seconds) = arg("--time").and_then(|s| s.parse::<f64>().ok()) {
        assert!(seconds.is_finite() && seconds >= 0.0);
        // Main-world time keeps aa-dev's FPS / screenshot-and-exit running. Only the
        // render-world clock is fixed, after extraction and before GPU resource upload.
        app.sub_app_mut(RenderApp).add_systems(
            Render,
            (move |mut time: ResMut<Time>| {
                *time = Time::default();
                time.advance_by(std::time::Duration::from_secs_f64(seconds));
            })
            .in_set(RenderSystems::PrepareAssets),
        );
    }
    app.run();
}

fn setup(mut commands: Commands, mut cry: CryAssets) {
    let asset = arg("--asset").unwrap_or_else(|| "size15".into());
    let (model, material, sub) = match asset.as_str() {
        "ma" => (
            "objects/env/02_harihara/01_ma/ma_waterfall_a.cgf",
            "objects/env/02_harihara/01_ma/ma_waterfall_a.mtl",
            1,
        ),
        "second" => (
            "objects/effects/waterfall/waterfall_size_15.cgf",
            "objects/env/05_nature/08_cry_source/ice/waterfall/waterfall_second.mtl",
            0,
        ),
        _ => (
            "objects/effects/waterfall/waterfall_size_15.cgf",
            "objects/effects/waterfall/waterfall_size_15.mtl",
            0,
        ),
    };
    let (center, distance) = if flag("--quad") {
        let handle = cry
            .material(Some(material), sub)
            .expect("original waterfall material");
        if flag("--unlit") {
            cry.materials.get_mut(&handle).unwrap().unlit = true;
        }
        commands.spawn((
            Mesh3d(cry.meshes.add(Rectangle::new(2.0, 2.0))),
            MeshMaterial3d(handle),
            Transform::default(),
        ));
        (Vec3::ZERO, 3.0)
    } else {
        let options = ModelOptions {
            material: Some(material.into()),
            material_hook: flag("--unlit")
                .then(|| aa_cry::MaterialHook::new(|_, m| m.unlit = true)),
            ..default()
        };
        let loaded = cry
            .load_model(model, &options)
            .expect("original waterfall mesh");
        aa_cry::scene::spawn_loaded(&mut commands, model, &loaded, &options);
        let (a, b) = loaded.model.bounds().expect("mesh bounds");
        let lo = Vec3::new(a.x, a.z, -b.y);
        let hi = Vec3::new(b.x, b.z, -a.y);
        ((lo + hi) * 0.5, (hi - lo).length() * 1.2)
    };
    let yaw = arg("--yaw")
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.0)
        .to_radians();
    let direction = Quat::from_rotation_y(yaw) * Vec3::Z;
    let camera =
        Transform::from_translation(center + direction * distance).looking_at(center, Vec3::Y);
    info!(
        "waterfall: model={model} material={material} sub={sub} quad={} camera={camera:?} time={:?}",
        flag("--quad"),
        arg("--time")
    );
    if !cry.cache.missing_textures.is_empty() {
        warn!(
            "waterfall missing textures: {:?}",
            cry.cache.missing_textures
        );
    }
    commands.spawn((
        Camera3d::default(),
        camera,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        bevy::camera::Exposure { ev100: 9.7 },
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, yaw - 0.7, -0.75, 0.0)),
    ));
}

fn token_zero(mut materials: ResMut<Assets<aa_cry::shade::illum::IllumMaterial>>) {
    if flag("--token-zero") {
        for (_, material) in materials.iter_mut() {
            material.extension.params.vertex.w = 0.0;
        }
    }
}
