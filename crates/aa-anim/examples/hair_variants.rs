//! Original hair style 24024: native default/none/type1..4 selector outputs.
//! No helmet policy is inferred: selectors and their DB-authored meshes are shown directly.
//! `--helmets` uses actual visible helmet items and `resolve_with_headgear`.
use aa_anim::hair::HairVariant;
use aa_anim::{
    AnimPlugin, Animator, CharacterDesc, ClipLibrary, GameDb, Gender, PartKind, Race,
    spawn_character,
};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-anim native hair variants"),
            aa_dev::DevPlugin,
            CryPlugin,
            AnimPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .insert_resource(GlobalAmbientLight {
            brightness: 600.0,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, freeze)
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let db = GameDb::open().unwrap();
    let part = db.body_part(24024).unwrap();
    let helmets = std::env::args().any(|a| a == "--helmets");
    let variants = if helmets {
        vec![
            HairVariant::Default,
            HairVariant::Type3,
            HairVariant::Hidden,
        ]
    } else {
        vec![
            HairVariant::Default,
            HairVariant::Hidden,
            HairVariant::Type1,
            HairVariant::Type2,
            HairVariant::Type3,
            HairVariant::Type4,
        ]
    };
    for (i, variant) in variants.into_iter().enumerate() {
        let mut desc = CharacterDesc::default_for(&db, Race::Nuian, Gender::Female).unwrap();
        desc.hair = Some(part.item_id);
        desc.hair_color = None;
        if helmets {
            desc.costume = aa_anim::Costume::None;
        }
        let helmet = if helmets {
            [None, Some(22874), Some(18230)][i]
        } else {
            None
        };
        if let Some(item) = helmet {
            desc.costume = aa_anim::Costume::Items(vec![item]);
        }
        let selector = match variant {
            HairVariant::Default => 0,
            HairVariant::Hidden => 1,
            HairVariant::Type1 => 2,
            HairVariant::Type2 => 3,
            HairVariant::Type3 => 4,
            HairVariant::Type4 => 5,
        };
        let detail = if let Some(item) = helmet {
            Some(db.item_visuals(item, 11).unwrap()[0].detail)
        } else if helmets {
            None
        } else {
            Some((3 << 13) | (selector << 10))
        };
        let mut assets = desc.resolve_with_headgear(&db, detail).unwrap();
        assets.parts.retain(|p| {
            matches!(p.kind, PartKind::Face | PartKind::Hair)
                || (helmets && p.kind == PartKind::Costume)
        });
        let x = (i % 3) as f32 * 0.52 - 0.52;
        let y = if helmets {
            0.0
        } else if i < 3 {
            0.32
        } else {
            -0.32
        };
        let transform = Transform::from_xyz(x, y, 0.0)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI));
        spawn_character(&mut commands, &mut cry, &clips, assets, transform).unwrap();
        info!(
            "native hair {:?}: asset {:?}, path {:?}",
            variant,
            part.variant_asset_id(variant),
            part.variant_path(variant)
        );
    }
    for (at, lux) in [
        (Vec3::new(1.5, 3.0, 5.0), 16000.0),
        (Vec3::new(-4.0, 2.0, 3.0), 4000.0),
    ] {
        commands.spawn((
            DirectionalLight {
                illuminance: lux,
                ..default()
            },
            Transform::from_translation(at).looking_at(Vec3::Y * 1.6, Vec3::Y),
        ));
    }
    commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Projection::Perspective(PerspectiveProjection {
            fov: if helmets { 20.0f32 } else { 24.0f32 }.to_radians(),
            ..default()
        }),
        Transform::from_xyz(0.0, 1.62, 3.2).looking_at(Vec3::new(0.0, 1.62, 0.0), Vec3::Y),
    ));
    let labels = if helmets {
        vec![
            "NO HELMET / NORMAL",
            "METAL039 / TYPE 3",
            "LEATHER031 / NONE",
        ]
    } else {
        vec!["DEFAULT", "NONE", "TYPE 1", "TYPE 2", "TYPE 3", "TYPE 4"]
    };
    for (i, text) in labels.into_iter().enumerate() {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(if helmets {
                    11.1 + i as f32 * 25.9
                } else {
                    15.5 + (i % 3) as f32 * 21.5
                }),
                width: percent(26.0),
                top: percent(if helmets {
                    90.0
                } else if i < 3 {
                    50.0
                } else {
                    95.0
                }),
                ..default()
            },
            Text::new(text),
            TextLayout::justify(Justify::Center),
            TextFont {
                font_size: FontSize::Px(22.0),
                ..default()
            },
        ));
    }
}

fn freeze(mut bodies: Query<&mut Animator>) {
    for mut body in &mut bodies {
        body.paused = true;
        body.random_idle = None;
        if let Some(current) = &mut body.current {
            current.time = 0.7;
        }
    }
}
