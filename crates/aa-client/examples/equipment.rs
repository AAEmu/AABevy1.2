//! Rigid equipment on the body: weapons on their sheath bones, backpacks and the body part slots (tail, beard, glasses), spawned
//! the way the game does it (`appearance::describe` / `resolve`, `weapons::carried` / `attach_carried`).
//!
//! `AA_EQUIP="race=nu,g=m,main=5660;race=fe,g=f,main=5961,back=23589"` lists the figures (`;`), each a list of `key=value`
//! (`race` nu / el / ha / fe, `g` m / f, and the slots `main`, `off`, `ranged`, `music`, `back` (backpack), `tail`, `beard`,
//! `glasses`, `chest`, `legs`, `feet`, `hands`, `head`, `cosplay` with an item template id; `label=` names it). `AA_EQUIP_VIEW`
//! is `back` (default), `left`, `right` or `front`, `AA_EQUIP_FOV` the vertical field of view in degrees,
//! `AA_EQUIP_DRAW=1` puts the weapons in the hands, `AA_EQUIP_CLIP=<clip of the .cal>` / `AA_EQUIP_TIME=<s>` play and hold a clip.
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
#[path = "../src/task.rs"]
mod task;
#[path = "../src/lua_value.rs"]
mod lua_value;
use lua_value::lua_int;

use aa_anim::{AnimPlugin, Animator, ClipLibrary, GameDb, Gender, Race, facial::FacialPlugin};
use aa_cry::scene::{CryAssets, CryPlugin};
use aa_net::model::{Equipment, Item, ModelParams, slot};
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
        .add_plugins((aa_dev::default_plugins("aa-client: equipment"), aa_dev::DevPlugin, CryPlugin, AnimPlugin, FacialPlugin))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .insert_resource(GlobalAmbientLight { brightness: 600., ..default() })
        .add_systems(Startup, setup)
        .add_systems(Update, (freeze, ingame::weapons::attach_carried, ingame::weapons::hold_weapons))
        .run();
}

fn item(template_id: u32) -> Item {
    Item { id: u64::from(template_id), template_id, count: 1, ..default() }
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let db = GameDb::open().expect("read-only original database");
    let client_db = ingame::data::Db::open().expect("game database");
    let spec = std::env::var("AA_EQUIP").unwrap_or_else(|_| "race=nu,g=m,main=5660".into());
    let figures: Vec<&str> = spec.split(';').filter(|f| !f.trim().is_empty()).collect();
    let view = std::env::var("AA_EQUIP_VIEW").unwrap_or_else(|_| "back".into());
    let fov: f32 = std::env::var("AA_EQUIP_FOV").ok().and_then(|v| v.parse().ok()).unwrap_or(25.);
    let spacing = 0.95;
    let n = figures.len() as f32;
    // the figures face +Z (as in the headgear example)
    let yaw = std::f32::consts::PI;
    for (i, figure) in figures.iter().enumerate() {
        let mut race = Race::Nuian;
        let mut gender = Gender::Male;
        let mut label = String::new();
        let mut equipment = Equipment::default();
        for kv in figure.split(',') {
            let Some((k, v)) = kv.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "race" => {
                    race = match v {
                        "el" => Race::Elf,
                        "ha" => Race::Hariharan,
                        "fe" => Race::Ferre,
                        _ => Race::Nuian,
                    }
                }
                "g" => gender = if v == "f" { Gender::Female } else { Gender::Male },
                "label" => label = v.to_string(),
                _ => {
                    let s = match k {
                        "main" => slot::MAINHAND,
                        "off" => slot::OFFHAND,
                        "ranged" => slot::RANGED,
                        "music" => slot::MUSICAL,
                        "back" => slot::BACKPACK,
                        "cloak" => slot::BACK,
                        "tail" => slot::TAIL,
                        "beard" => slot::BEARD,
                        "glasses" => slot::GLASSES,
                        "chest" => slot::CHEST,
                        "legs" => slot::LEGS,
                        "feet" => slot::FEET,
                        "hands" => slot::HANDS,
                        "head" => slot::HEAD,
                        "cosplay" => slot::COSPLAY,
                        _ => continue,
                    };
                    equipment.slots[s] = v.parse().ok().map(item);
                }
            }
        }
        if label.is_empty() {
            label = figure.to_string();
        }
        let desc = ingame::appearance::describe(&db, race, gender, &equipment, &ModelParams::default()).expect("describe");
        let assets = ingame::appearance::resolve(&db, &desc, &equipment).expect("resolve");
        let carried = ingame::weapons::carried(&client_db, &equipment);
        info!("equipment: {label}: {} parts, carries {:?}", assets.parts.len(), carried.iter().map(|c| (c.bone, c.path.rsplit('/').next().unwrap_or("").to_string())).collect::<Vec<_>>());
        // labels run left to right on screen: the back camera looks along +Z, where +X is on the left (the left view: +Z)
        let x = (i as f32 - (n - 1.) / 2.) * spacing * if matches!(view.as_str(), "back" | "left" | "side") { -1. } else { 1. };
        let pos = if matches!(view.as_str(), "side" | "left" | "right") { Vec3::new(0., 0., x) } else { Vec3::new(x, 0., 0.) };
        let cal = aa_anim::Cal::load(&assets.cal).ok().map(std::sync::Arc::new);
        let root = ingame::player::spawn_with_locomotion(&mut commands, &mut cry, &clips, assets, Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)), carried)
            .expect("actual client player spawn");
        if let Some(cal) = cal {
            commands.entity(root).insert(FigureCal(cal));
        }
        commands.spawn((
            Node { position_type: PositionType::Absolute, left: percent(i as f32 * 100. / n), width: percent(100. / n), top: percent(92.), ..default() },
            Text::new(label),
            TextLayout::justify(Justify::Center),
            TextFont { font_size: FontSize::Px(16.), ..default() },
        ));
    }
    let span = (n * spacing).max(1.6);
    let dist = (span / 2.0) / (fov.to_radians() / 2.0).tan() * 1.15 / (16. / 9.) * 1.0;
    let dist = dist.max(3.2);
    let (eye, up) = match view.as_str() {
        "left" | "side" => (Vec3::new(dist, 1.1, 0.), Vec3::Y),
        "right" => (Vec3::new(-dist, 1.1, 0.), Vec3::Y),
        "front" => (Vec3::new(0., 1.1, dist), Vec3::Y),
        _ => (Vec3::new(0., 1.1, -dist), Vec3::Y),
    };
    commands.spawn((
        Camera3d::default(),
        aa_cry::stage::login_color_grading(),
        Projection::Perspective(PerspectiveProjection { fov: fov.to_radians(), ..default() }),
        Transform::from_translation(eye).looking_at(Vec3::new(0., 1.0, 0.), up),
    ));
}

#[derive(Component)]
struct FigureCal(std::sync::Arc<aa_anim::Cal>);

fn freeze(mut commands: Commands, clips: Res<ClipLibrary>, mut figures: Query<(Entity, &mut Animator, Option<&FigureCal>)>, mut started: Local<bool>) {
    // `AA_EQUIP_CLIP=fist_fishing_idle`: a clip of the figure's `.cal` instead of the idle, held at `AA_EQUIP_TIME` seconds
    let clip = std::env::var("AA_EQUIP_CLIP").ok();
    let time: f32 = std::env::var("AA_EQUIP_TIME").ok().and_then(|t| t.parse().ok()).unwrap_or(0.7);
    // `AA_EQUIP_DRAW=1`: the weapons are in the hands
    let draw = std::env::var_os("AA_EQUIP_DRAW").is_some();
    for (entity, mut body, cal) in &mut figures {
        if let (Some(name), Some(cal), false) = (&clip, cal, *started) {
            if let Some(entry) = cal.0.animation(name) {
                let options = aa_anim::pose::SampleOptions { position_scale: entry.position_scale(cal.0.base_height), lock_root_motion: true };
                if let Ok(bound) = clips.bound(&body.rig, &entry.path, options) {
                    body.play(bound, true, 0.0);
                } else {
                    warn!("equipment: clip {name} does not bind");
                }
            } else {
                warn!("equipment: no clip {name}");
            }
        }
        if draw {
            commands.entity(entity).try_insert(ingame::weapons::Drawn { until: f64::MAX, out: ingame::weapons::Out::Blade { dual: false }, sheathing: false });
        }
        body.paused = true;
        body.random_idle = None;
        if let Some(current) = &mut body.current {
            current.time = time;
        }
    }
    if clip.is_some() && !figures.is_empty() {
        *started = true;
    }
}
