//! The summon emergence of a newly created slave (the owner test's `/slave spawn 146`, the Red Farm Hauler):
//! `SCSlaveCreatedPacket` without `hideSpawnEffect` starts the unit two bounding extents behind its position, behind
//! the portal plane, and moves it out over `portal_time` 1 s (`FUN_3909ea30` / `FUN_39097220`); the portal
//! (`slaves.portal_spawn_fx_id` 1263 `portal_tank_spawn`, scale 3) hangs at the plane. A vehicle the server only
//! streams in later (no creation packet) and a creation with `hideSpawnEffect` play none.
//!
//! `cargo run -p aa-client --example slave_summon -- --mute --offscreen-shot shots/aa-client/slave_summon.png --shot-after 2.2`
//!
//! The creation packet is sent once the scene is drawn (1.4 s in), so the screencap sees the cart emerging through its
//! portal. Without `--shot*` the example checks the start/plane, the end after `portal_time`, and the particles
//! fading out, and exits non-zero on a mismatch.

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

use aa_cry::shade::illum::IllumMaterial;
use bevy::math::Vec4;
use bevy::pbr::MeshMaterial3d;
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
/// When the creation packet is sent (after the carts and their textures are drawn), when the emergence is checked,
/// when it must have ended (`portal_time` 1.0) and when the effect particles must be gone.
const ANNOUNCE_AT: f32 = 1.4;
const DRAWN_AT: f32 = 1.7;
const ENDED_AT: f32 = 3.0;
const GONE_AT: f32 = 6.0;

#[derive(Resource, Default)]
struct Check {
    start: Option<Instant>,
    announced: bool,
    drawn: bool,
    ended: bool,
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
            aa_fx::FxPlugin,
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
                ingame::vehicle::animate_summon_emergence,
                ingame::vehicle::apply_emergence_clip,
                ingame::vehicle::end_summon_portals,
                ingame::vehicle::despawn_summon_portal_effects,
                ingame::vehicle::restore_emergence_clip,
                ingame::vehicle::cleanup_summon_portals,
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

#[allow(clippy::too_many_arguments)]
fn check(
    mut check: ResMut<Check>,
    mut writer: MessageWriter<aa_net::NetEvent>,
    effects: Query<(&aa_fx::Effect, &ingame::vehicle::SummonPortalEffect)>,
    portal_fx: Query<&aa_fx::Effect>,
    roots: Query<(&Transform, &ingame::vehicle::SummonEmergence)>,
    anchors: Query<(&ingame::vehicle::SummonPortalAnchor, &Transform)>,
    illum: Option<Res<Assets<IllumMaterial>>>,
    materials: Query<&MeshMaterial3d<IllumMaterial>>,
    mut exit: MessageWriter<AppExit>,
) {
    let start = *check.start.get_or_insert_with(Instant::now);
    let elapsed = start.elapsed().as_secs_f32();
    // `SCSlaveCreatedPacket` for the first cart only; the second one is the streamed-in control. Sent after the scene is
    // up so a screencap sees the cart emerging through its portal.
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
        let (transform, emergence) = roots.single().expect("the summoned cart's emergence");
        assert!(!emergence.ended, "the emergence is still running");
        let path = (emergence.end - emergence.start).length();
        let progress = (transform.translation - emergence.start).length() / path;
        assert!(progress > 0.0 && progress < 1.0, "the cart is emerging: {progress}");
        assert!((emergence.plane - (emergence.end - emergence.axis * emergence.extent)).length() < 1e-3);
        let (_, anchor) = anchors.single().expect("the portal anchor");
        assert!((anchor.translation - emergence.plane).length() < 1e-3, "{anchor:?}");
        let clipped = illum
            .as_deref()
            .map(|illum| {
                materials
                    .iter()
                    .filter_map(|handle| illum.get(&handle.0))
                    .filter(|material| material.extension.clip_plane != Vec4::ZERO)
                    .count()
            })
            .unwrap_or(0);
        assert!(clipped > 0, "the emerging cart's Illum materials are clipped at the plane");
        info!(
            "slave_summon: emergence start {:?} plane {:?} end {:?}, portal {}",
            emergence.start,
            emergence.plane,
            emergence.end,
            names.join(", ")
        );
        check.drawn = true;
    }
    if check.drawn && !check.ended && elapsed >= ENDED_AT {
        let (transform, emergence) = roots.single().expect("the summoned cart's emergence");
        assert!(emergence.ended, "the emergence ends at portal_time");
        assert!((transform.translation - emergence.end).length() < 1e-3, "{:?}", transform.translation);
        assert!(
            effects.iter().all(|(_, marker)| marker.stopped),
            "the portal stops emitting at the end event 5"
        );
        let clipped = illum
            .as_deref()
            .map(|illum| {
                materials
                    .iter()
                    .filter_map(|handle| illum.get(&handle.0))
                    .filter(|material| material.extension.clip_plane != Vec4::ZERO)
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(clipped, 0, "the clip plane is restored after portal_time");
        info!("slave_summon: emergence ended at {:?}", emergence.end);
        check.ended = true;
    }
    if check.ended && !check.gone && elapsed >= GONE_AT {
        let left: Vec<&str> = portal_fx.iter().map(|effect| effect.name.as_str()).filter(|name| name.contains("portal_tank")).collect();
        assert!(left.is_empty(), "the portal particles fade out: {left:?}");
        assert!(effects.is_empty(), "the capped portal effects are despawned");
        info!("slave_summon: ok (emergence, portal end and the streamed-in control match 1.2)");
        check.gone = true;
        if !aa_dev::args()
            .iter()
            .any(|a| matches!(a.as_str(), "--shot" | "--offscreen-shot" | "--exit-after"))
        {
            exit.write(AppExit::Success);
        }
    }
}
