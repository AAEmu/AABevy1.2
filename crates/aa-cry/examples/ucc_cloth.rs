//! Static sail composition/control fixture. `--shot shots/aa-cry/ucc_cloth.png --shot-after 8`.
//! Unlit, white material factors isolate texture composition from cloth lighting and wind.

use aa_cry::{CryAssets, CryPlugin, MaterialHook, ModelOptions};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;

const MODEL: &str = "objects/env/06_unit/01_ship/galleon/galleon_deco_sail01.cgf";
const MATERIAL: &str = "objects/env/06_unit/01_ship/galleon/galleon_mahadevi.mtl";
const PATTERN: &str = "objects/env/07_ucc/ucc_pattern04.dds";
const LOGO: &str = "objects/env/07_ucc/ucc_logo32.dds";

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-cry: authored UCC cloth"),
            aa_dev::DevPlugin,
            CryPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.12, 0.14, 0.17)))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets) {
    let raw = cry.texture(PATTERN, true).unwrap();
    let mark = cry.texture(LOGO, false).unwrap();
    let converted = cry.material(Some(MATERIAL), 3).unwrap();
    let composed = cry
        .materials
        .get(&converted)
        .unwrap()
        .base_color_texture
        .clone()
        .unwrap();
    let mut bounds = None;
    for (i, raw_texture) in [Some(raw.clone()), None].into_iter().enumerate() {
        let options = ModelOptions {
            material: Some(MATERIAL.into()),
            material_hook: Some(MaterialHook::new(move |m, out| {
                out.unlit = true;
                out.base_color = Color::WHITE;
                if m.name == "ucc" {
                    if let Some(texture) = &raw_texture {
                        out.base_color_texture = Some(texture.clone());
                    }
                }
            })),
            ..default()
        };
        let loaded = cry.load_model(MODEL, &options).unwrap();
        let (lo, hi) = loaded.model.bounds().unwrap();
        let a = Vec3::new(lo.x, lo.z, -hi.y);
        let b = Vec3::new(hi.x, hi.z, -lo.y);
        let centre = (a + b) * 0.5;
        let span = b - a;
        let spacing = span.x.max(span.z) * 0.7;
        let spawned = cry
            .spawn_model_full(&mut commands, MODEL, &options)
            .unwrap();
        let offset = Vec3::X * if i == 0 { -spacing } else { spacing } - centre;
        commands
            .entity(spawned.root)
            .insert(Transform::from_translation(offset).with_rotation(aa_cry::CRY_TO_BEVY));
        bounds = Some((span, spacing));
        info!(
            "ucc_cloth: {i}: span={span:?}, material={:?}",
            loaded.material_path
        );
    }
    let (span, spacing) = bounds.unwrap();
    let radius = (span.length() * 0.5 + spacing).max(1.0);
    let target = Vec3::Y * (-span.y * 0.17);
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Transform::from_translation(target + Vec3::new(radius * 0.12, radius * 0.05, radius * 2.0))
            .looking_at(target, Vec3::Y),
    ));
    commands.spawn((
        Node { position_type: PositionType::Absolute, left: px(30), top: px(24), ..default() },
        Text::new("Original mesh + raw Diffuse (control)                                  Original mesh + authored static UCC composition"),
        TextFont { font_size: FontSize::Px(23.0), ..default() },
        TextColor(Color::WHITE),
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(30), bottom: px(24), column_gap: px(24),
            align_items: AlignItems::Start, ..default()
        },
    )).with_children(|parent| {
        for (label, image) in [("Authored RGB pattern (raw)", raw), ("Static composition (linear)", composed), ("Authored crest (linear)", mark)] {
            parent.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(8), ..default() }).with_children(|panel| {
                panel.spawn((Text::new(label), TextFont { font_size: FontSize::Px(18.0), ..default() }, TextColor(Color::WHITE)));
                panel.spawn((ImageNode::new(image), Node { width: px(210), height: px(210), ..default() }, BackgroundColor(Color::srgb(0.3, 0.3, 0.3))));
            });
        }
        parent.spawn((
            Node { width: px(620), margin: UiRect::left(px(12)), ..default() },
            Text::new("galleon_mahadevi.mtl / submaterial 3\nRed mask = 0.21568629 grey; green / blue = white\nCrest UV = mesh UV * 3 - 1; strict 0 < UV < 1\nDiffuse alpha retained; unlit texture comparison\nStatic fallback: authored Diffuse pattern supplies $CustomMap"),
            TextFont { font_size: FontSize::Px(19.0), ..default() },
            TextColor(Color::srgb(0.8, 0.84, 0.9)),
        ));
    });
}
