//! Received equipment -> client appearance resolver -> actual player character spawn.
#![allow(dead_code)]
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;

use aa_anim::{AnimPlugin, Animator, ClipLibrary, GameDb, Gender, Race, facial::FacialPlugin};
use aa_cry::scene::{CryAssets, CryPlugin};
use aa_net::model::{EquipDetail, Equipment, Item, ItemDetail, ModelParams, slot};
use bevy::prelude::*;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    WorldSelect,
    Login,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: received headgear hair variants"),
            aa_dev::DevPlugin,
            CryPlugin,
            AnimPlugin,
            FacialPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .insert_resource(GlobalAmbientLight {
            brightness: 600.,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, freeze)
        .run();
}
fn item(template_id: u32) -> Item {
    Item {
        id: u64::from(template_id),
        template_id,
        count: 1,
        ..default()
    }
}
fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let db = GameDb::open().expect("read-only original database");
    for (i, (label, helmet, image)) in [
        ("NORMAL HAIR", None, 0),
        ("METAL / TYPE 3", Some(22874), 0),
        ("HOOD / HIDDEN HAIR", Some(18230), 0),
        ("IMAGE METAL / TYPE 3", Some(18230), 22874),
    ]
    .into_iter()
    .enumerate()
    {
        let mut equipment = Equipment::default();
        equipment.slots[slot::HAIR] = Some(item(24024));
        equipment.slots[slot::CHEST] = Some(item(2691));
        equipment.slots[slot::HEAD] = helmet.map(|id| Item {
            detail: ItemDetail::Equip(EquipDetail {
                image_item_template_id: image,
                ..default()
            }),
            ..item(id)
        });
        let desc = ingame::appearance::describe(&db, Race::Nuian, Gender::Female, &equipment, &ModelParams::default()).unwrap();
        let assets = ingame::appearance::resolve(&db, &desc, &equipment).unwrap();
        assert!(matches!(&desc.costume, aa_anim::Costume::Items(ids) if ids.contains(&2691)));
        let hair = assets.parts.iter().find(|part| part.kind == aa_anim::PartKind::Hair);
        if i == 2 {
            assert!(hair.is_none());
        } else {
            assert!(hair.is_some());
        }
        info!(
            "headgear {label}: helmet={helmet:?} image={image} hair={:?}",
            hair.map(|part| &part.path)
        );
        ingame::player::spawn_with_locomotion(
            &mut commands,
            &mut cry,
            &clips,
            assets,
            Transform::from_xyz((i as f32 - 1.5) * 0.53, 0., 0.).with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
            vec![],
        )
        .expect("actual client player spawn");
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(i as f32 * 25.),
                width: percent(25.),
                top: percent(90.),
                ..default()
            },
            Text::new(label),
            TextLayout::justify(Justify::Center),
            TextFont {
                font_size: FontSize::Px(20.),
                ..default()
            },
        ));
    }
    commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Projection::Perspective(PerspectiveProjection {
            fov: 25f32.to_radians(),
            ..default()
        }),
        Transform::from_xyz(0., 1.62, 3.2).looking_at(Vec3::new(0., 1.62, 0.), Vec3::Y),
    ));
}
fn freeze(mut figures: Query<&mut Animator>) {
    for mut body in &mut figures {
        body.paused = true;
        body.random_idle = None;
        if let Some(current) = &mut body.current {
            current.time = 0.7;
        }
    }
}
