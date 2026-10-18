//! Original cat CDF geometry/material: old smallest-mip elision versus authored cutouts.
//! `--shot shots/aa-cry/alpha_cutout.png --shot-after 8`; unlit isolates alpha coverage.
use aa_cry::material::TextureMap;
use aa_cry::{CryAssets, CryPlugin, MaterialHook, ModelOptions};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;

const CDF: &str = "objects/characters/animals/cat_baby_pet/cat_baby_black_pet.cdf";

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-cry: authored alpha cutouts"),
            aa_dev::DevPlugin,
            CryPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.16, 0.22, 0.3)))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets) {
    let bytes = aa_cry::read(CDF).unwrap();
    let source = std::str::from_utf8(&bytes).unwrap();
    let cdf = roxmltree::Document::parse(source).unwrap();
    let model = cdf.descendants().find(|n| n.has_tag_name("Model")).unwrap();
    let path = model.attribute("File").unwrap();
    let material_path = model.attribute("Material").unwrap();
    let mtl = cry.mtl(material_path).unwrap();
    let m = mtl.sub(0).unwrap();
    let texture_path = m.texture(&TextureMap::Diffuse).unwrap().pak_path();
    let raw = cry.texture(&texture_path, true).unwrap();
    let loader = cry.loader();
    assert_eq!(loader.mean_alpha(&texture_path), Some(1.0));
    assert_eq!(loader.alpha_is_opaque(&texture_path), Some(false));
    let converted = cry.material(Some(material_path), 0).unwrap();
    assert_eq!(
        cry.materials.get(&converted).unwrap().alpha_mode,
        AlphaMode::Mask(0.5)
    );
    let rotation = Quat::from_rotation_y(std::f32::consts::PI - 0.35) * aa_cry::CRY_TO_BEVY;
    let options = ModelOptions {
        material: Some(material_path.into()),
        ..default()
    };
    let loaded = cry.load_model(path, &options).unwrap();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for mesh in &loaded.model.meshes {
        for p in &mesh.positions {
            let p = rotation * *p;
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    let center = (lo + hi) * 0.5;
    let extent = hi - lo;
    let spacing = extent.x.max(extent.y) * 0.9;
    for i in 0..2 {
        let options = ModelOptions {
            material_hook: Some(MaterialHook::new(move |_, out| {
                out.unlit = true;
                out.base_color = Color::WHITE;
                if i == 0 {
                    out.alpha_mode = AlphaMode::Opaque;
                }
            })),
            ..options.clone()
        };
        let spawned = cry.spawn_model_full(&mut commands, path, &options).unwrap();
        commands.entity(spawned.root).insert(
            Transform::from_translation(Vec3::X * if i == 0 { -spacing } else { spacing } - center)
                .with_rotation(rotation),
        );
    }
    info!("alpha_cutout: model={path}, material={material_path}, bounds={lo:?}..{hi:?}");
    let focus = Vec3::Y * (-extent.y * 0.13);
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical { viewport_height: extent.y * 2.2 },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Transform::from_translation(focus + Vec3::Z * (extent.y.max(extent.x) * 3.4))
            .looking_at(focus, Vec3::Y),
    ));
    commands.spawn((Node {position_type:PositionType::Absolute,left:px(30),top:px(24),..default()},
        Text::new("Original cat / old opaque elision                             Original cat / authored AlphaTest 0.5"),
        TextFont {font_size:FontSize::Px(23.0),..default()},TextColor(Color::WHITE)));
    commands.spawn(Node {position_type:PositionType::Absolute,left:px(30),bottom:px(24),column_gap:px(24),..default()}).with_children(|p| {
        p.spawn((ImageNode::new(raw),Node {width:px(240),height:px(240),..default()}));
        p.spawn((Text::new("Original cat_baby_black_pet.cdf mesh and material\nSmallest authored mip: alpha 255 everywhere\nBase mip: 2,192 texels below the authored threshold\nKeep the alpha test unless every authored mip proves opacity\nWhite/unlit factors isolate coverage; no fur lighting changes"),
            TextFont {font_size:FontSize::Px(21.0),..default()},TextColor(Color::WHITE)));
    });
}
