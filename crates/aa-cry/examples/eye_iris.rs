//! Original Ferre head mesh: raw Eye albedo versus the authored static IrisColor operation.
//! `--shot shots/aa-cry/eye_iris.png --shot-after 8`; white/unlit factors isolate albedo.
use aa_cry::material::TextureMap;
use aa_cry::{CryAssets, CryPlugin, MaterialHook, ModelOptions};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;

const MODEL: &str = "objects/characters/ferre/female/face/face01/fe_f_face01.chr";
const MATERIAL: &str = "objects/characters/ferre/female/face/face01/fe_f_face01.mtl";

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-cry: authored Eye iris"),
            aa_dev::DevPlugin,
            CryPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.1, 0.12, 0.15)))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets) {
    let mtl = cry.mtl(MATERIAL).unwrap();
    let eye = mtl.sub(3).unwrap();
    let raw = cry
        .texture(&eye.texture(&TextureMap::Diffuse).unwrap().pak_path(), true)
        .unwrap();
    let gloss = cry
        .texture(
            &eye.texture(&TextureMap::Specular).unwrap().pak_path(),
            true,
        )
        .unwrap();
    let material = cry.material(Some(MATERIAL), 3).unwrap();
    let composed = cry
        .materials
        .get(&material)
        .unwrap()
        .base_color_texture
        .clone()
        .unwrap();
    assert_ne!(
        raw, composed,
        "the source Eye material must take the iris composition path"
    );
    let rotation = Quat::from_rotation_y(std::f32::consts::PI) * aa_cry::CRY_TO_BEVY;
    for i in 0..2 {
        let control = (i == 0).then_some(raw.clone());
        let options = ModelOptions {
            material: Some(MATERIAL.into()),
            material_hook: Some(MaterialHook::new(move |m, out| {
                out.unlit = true;
                out.base_color = Color::WHITE;
                if m.shader.eq_ignore_ascii_case("eye")
                    && let Some(raw) = &control
                {
                    out.base_color_texture = Some(raw.clone());
                }
            })),
            ..default()
        };
        let loaded = cry.load_model(MODEL, &options).unwrap();
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for mesh in &loaded.model.meshes {
            for s in mesh.subsets.iter().filter(|s| s.material_id == 3) {
                for &id in
                    &mesh.indices[s.first_index as usize..(s.first_index + s.index_count) as usize]
                {
                    let p = rotation * mesh.positions[id as usize];
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
        }
        assert!(lo.is_finite() && hi.is_finite() && hi.x > lo.x);
        let focus = (lo + hi) * 0.5;
        let spawn = cry
            .spawn_model_full(&mut commands, MODEL, &options)
            .unwrap();
        commands.entity(spawn.root).insert(
            Transform::from_translation(Vec3::X * if i == 0 { -0.21 } else { 0.21 } - focus)
                .with_rotation(rotation),
        );
        info!(
            "eye_iris: {i}: eye bounds={lo:?}..{hi:?}, source IrisColor={:?}",
            eye.public_param("IrisColor")
        );
    }
    let focus = Vec3::new(0.0, -0.04, 0.0);
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        DebandDither::Disabled,
        Transform::from_translation(focus + Vec3::Z * 0.85).looking_at(focus, Vec3::Y),
    ));
    commands.spawn((Node { position_type: PositionType::Absolute, left:px(30),top:px(24),..default() },
        Text::new("Original head / raw Eye albedo (control)                        Original head / authored IrisColor"),
        TextFont {font_size:FontSize::Px(23.0),..default()},TextColor(Color::WHITE)));
    commands.spawn(Node {position_type:PositionType::Absolute,left:px(30),bottom:px(24),column_gap:px(24),..default()}).with_children(|p| {
        for (label,image) in [("Original colour atlas",raw),("Gloss mask (sRGB red)",gloss),("Eye albedo atlas",composed)] {
            p.spawn(Node {flex_direction:FlexDirection::Column,row_gap:px(8),..default()}).with_children(|p| {
                p.spawn((Text::new(label),TextFont {font_size:FontSize::Px(18.0),..default()},TextColor(Color::WHITE)));
                p.spawn((ImageNode::new(image),Node {width:px(210),height:px(210),..default()}));
            });
        }
        p.spawn((Node {width:px(680),margin:UiRect::left(px(12)),..default()},
            Text::new("Ferre female face01 / Eye submaterial 3\nIrisColor = 0.38039219, 0.50196081, 0.627451\nDiffuse *= lerp(1, IrisColor, sRGB-decode(gloss.r))\nColour 512x512 / gloss 256x256; alpha preserved\nStatic authored material; custom face hooks can replace it"),
            TextFont {font_size:FontSize::Px(19.0),..default()},TextColor(Color::srgb(0.8,0.84,0.9))));
    });
}
