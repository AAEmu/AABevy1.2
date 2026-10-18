//! Original hair style 24024: native default/none/type1..4 selector outputs.
//! No helmet policy is inferred: selectors and their DB-authored meshes are shown directly.
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
    for (i, variant) in [
        HairVariant::Default,
        HairVariant::Hidden,
        HairVariant::Type1,
        HairVariant::Type2,
        HairVariant::Type3,
        HairVariant::Type4,
    ]
    .into_iter()
    .enumerate()
    {
        let mut desc = CharacterDesc::default_for(&db, Race::Nuian, Gender::Female).unwrap();
        desc.hair = Some(part.item_id);
        desc.hair_color = None;
        let mut assets = desc.resolve(&db).unwrap();
        assets
            .parts
            .retain(|p| matches!(p.kind, PartKind::Face | PartKind::Hair));
        match part.variant_path(variant) {
            Some(path) => {
                assets
                    .parts
                    .iter_mut()
                    .find(|p| p.kind == PartKind::Hair)
                    .unwrap()
                    .path = path.into()
            }
            None => assets.parts.retain(|p| p.kind != PartKind::Hair),
        }
        let x = (i % 3) as f32 * 0.52 - 0.52;
        let y = if i < 3 { 0.32 } else { -0.32 };
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
            fov: 24.0f32.to_radians(),
            ..default()
        }),
        Transform::from_xyz(0.0, 1.62, 3.2).looking_at(Vec3::new(0.0, 1.62, 0.0), Vec3::Y),
    ));
    for (i, text) in ["DEFAULT", "NONE", "TYPE 1", "TYPE 2", "TYPE 3", "TYPE 4"]
        .into_iter()
        .enumerate()
    {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(15.5 + (i % 3) as f32 * 21.5),
                width: percent(26.0),
                top: percent(if i < 3 { 50.0 } else { 95.0 }),
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
