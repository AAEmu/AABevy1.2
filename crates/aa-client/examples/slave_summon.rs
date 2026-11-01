//! The summon portal of a newly created slave (the owner test's `/slave spawn 146`, the Red Farm Hauler): the
//! `slaves.portal_spawn_fx_id` group 1263 `portal_tank_spawn` plays at the cart when `SCSlaveCreatedPacket` arrives
//! without `hideSpawnEffect` (`FUN_39349600`), scaled by `portal_scale` 3 and lasting `portal_time` 1 s. A vehicle the
//! server only streams in later (no creation packet) and a creation with `hideSpawnEffect` play none.
//!
//! `cargo run -p aa-client --example slave_summon -- --mute --offscreen-shot shots/aa-client/slave_summon.png --shot-after 2.0`
//!
//! The creation packet is sent once the scene is drawn (1.4 s in), so the screencap sees the cart and its portal.
//! Without `--shot*` the example checks the portal 0.3 s after the packet, checks it is gone after `portal_time`, and
//! exits non-zero on a mismatch.

#![allow(dead_code)]
#[path = "../src/graphics.rs"]
mod graphics;
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/logging.rs"]
mod logging;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/lua_value.rs"]
mod lua_value;
#[path = "../src/priority.rs"]
mod priority;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;

use bevy::prelude::*;
use std::time::Instant;

use ingame::vehicle::SummonPortals;
use lua_value::lua_int;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    Login,
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

/// The owner test's slave template and its model; the same cart is used for the streamed-in control.
const SLAVE: u32 = 146;
const MODEL: u32 = 1553;
const SUMMONED: u32 = 901;
const STREAMED: u32 = 902;
const GAP: f32 = 9.0;
/// When the creation packet is sent (after the carts and their textures are drawn), when to check the portal and
/// when it must be gone (just past `portal_time` 1.0).
const ANNOUNCE_AT: f32 = 1.4;
const DRAWN_AT: f32 = 1.7;
const GONE_AT: f32 = 3.0;

#[derive(Resource, Default)]
struct Check {
    start: Option<Instant>,
    announced: bool,
    drawn: bool,
    gone: bool,
}

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("slave_summon: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)");
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!("slave_summon: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)");
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("slave_summon: skipped, the game database cannot be opened");
        return AppExit::Success;
    };
    let Some(book) = ingame::combat::skills::SkillBook::open() else {
        eprintln!("slave_summon: skipped, the skill database cannot be opened");
        return AppExit::Success;
    };
    let portal = db.slave_summon_portal(SLAVE).expect("slave 146's summon portal");
    assert_eq!(portal.fx_group, 1263, "portal_tank_spawn");
    assert_eq!((portal.scale, portal.time), (3.0, 1.0));
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Slave summon portal"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_world::pipelines::build,
        ))
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(db)
        .insert_resource(book)
        .init_resource::<aa_net::state::Units>()
        .init_resource::<SummonPortals>()
        .init_resource::<Check>()
        .add_message::<aa_net::NetEvent>()
        .add_systems(Startup, (stage, scene).chain())
        .add_systems(
            Update,
            (
                ingame::vehicle::track_summon_portals,
                ingame::vehicle::spawn_summon_portals,
                ingame::vehicle::despawn_summon_portals,
                check,
            )
                .chain(),
        )
        .run()
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let width = GAP * 3.0;
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(width * 0.5, 7.0, 12.0)
            .looking_at(Vec3::new(width * 0.5 - GAP * 0.5, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(width + 40.0, 80.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.16, 0.19, 0.21))),
        Transform::from_xyz(width * 0.5, 0.0, 0.0),
    ));
}

fn slave_unit(obj_id: u32) -> aa_net::model::Unit {
    aa_net::model::Unit {
        obj_id,
        name: format!("red farm hauler {obj_id}"),
        kind: aa_net::model::UnitKind::Slave {
            id: SLAVE,
            tl_id: obj_id as u16,
            template_id: SLAVE,
            summoner_id: 1,
        },
        owner_name: String::new(),
        pos: aa_net::Pos::new(0.0, 0.0, 0.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 50,
        model_id: MODEL,
        equipment: aa_net::model::Equipment::default(),
        model_params: aa_net::model::ModelParams::default(),
        hp: 100,
        mp: 0,
        max_hp: 100,
        max_mp: 0,
        dead: false,
        race_gender: 0,
        movement: None,
        faction_id: 0,
        expedition_id: 0,
        appellation: 0,
        flags: 0,
        gm_modes: [0; 4],
        bounty_money: None,
        pvp_honor: 0,
        pvp_kills: 0,
        skills: Vec::new(),
        passive_buffs: Vec::new(),
        ability_exp: Vec::new(),
        active_abilities: Vec::new(),
        buffs: Vec::new(),
        anim_action: 0,
        house_posture: None,
        turret: None,
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
    }
}

fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    db: Res<ingame::data::Db>,
    mut units: ResMut<aa_net::state::Units>,
) {
    let look = db.vehicle_look(MODEL).expect("the Red Farm Hauler's look");
    let max_hp = db.slave_max_hp(SLAVE, 50).expect("slave 146's MaxHealth");
    let loader = cry.loader();
    let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    for (i, obj_id) in [SUMMONED, STREAMED].into_iter().enumerate() {
        let unit = slave_unit(obj_id);
        let pieces = ingame::vehicle::prepare(&loader, &look);
        assert!(!pieces.is_empty(), "the Red Farm Hauler has parts");
        let transform = Transform::from_xyz(i as f32 * GAP, 0.0, 0.0).with_rotation(rotation);
        let root = ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, transform);
        commands.entity(root).insert((
            ingame::units::RemoteUnit::announced(&unit),
            ingame::vehicle::VehicleDamage {
                state: 1,
                variant: look.normal.clone(),
                max_hp,
                requested: None,
                dying_start: None,
                dying_time: 0.0,
                death_seen: false,
                dead_effect_played: false,
            },
        ));
        units.by_id.insert(obj_id, unit);
    }
}

fn check(
    mut check: ResMut<Check>,
    mut writer: MessageWriter<aa_net::NetEvent>,
    effects: Query<(&aa_fx::Effect, &ingame::vehicle::SummonPortalEffect)>,
    mut exit: MessageWriter<AppExit>,
) {
    let start = *check.start.get_or_insert_with(Instant::now);
    let elapsed = start.elapsed().as_secs_f32();
    // `SCSlaveCreatedPacket` for the first cart only; the second one is the streamed-in control. Sent after the scene is
    // up so a screencap sees the cart under its portal.
    if !check.announced && elapsed >= ANNOUNCE_AT {
        writer.write(aa_net::NetEvent(aa_net::Event::Slave(
            aa_net::slave::SlaveEvent::Created(aa_net::slave::SlaveCreated {
                owner_obj_id: 1,
                tl_id: SUMMONED as u16,
                obj_id: SUMMONED,
                hide_spawn_effect: false,
                raw_id: 0,
                creator_name: "owner".into(),
            }),
        )));
        info!("slave_summon: SCSlaveCreatedPacket for cart {SUMMONED}");
        check.announced = true;
    }
    if check.announced && !check.drawn && elapsed >= DRAWN_AT {
        let names: Vec<&str> = effects.iter().map(|(effect, _)| effect.name.as_str()).collect();
        assert_eq!(names.len(), 2, "the summoned cart's portal: {names:?}");
        assert!(names.iter().any(|name| name.ends_with("portal_tank_main")), "{names:?}");
        assert!(names.iter().any(|name| name.ends_with("portal_tank_main_start")), "{names:?}");
        info!("slave_summon: portal {}", names.join(", "));
        check.drawn = true;
    }
    if check.drawn && !check.gone && elapsed >= GONE_AT {
        let names: Vec<&str> = effects.iter().map(|(effect, _)| effect.name.as_str()).collect();
        assert!(names.is_empty(), "the portal outlives portal_time: {names:?}");
        info!("slave_summon: ok (the streamed-in cart and the portal expiry match 1.2)");
        check.gone = true;
        if !aa_dev::args()
            .iter()
            .any(|a| matches!(a.as_str(), "--shot" | "--offscreen-shot" | "--exit-after"))
        {
            exit.write(AppExit::Success);
        }
    }
}
