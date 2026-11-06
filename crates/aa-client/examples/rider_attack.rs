//! A seated rider's attack: Mounted Attack (Rider) 11013 fires `onehand_co_horse_attack_r_slash` (anim 84) on a Nuian man
//! riding the Lilyut horse (model 139), with no server. The game's own systems do the work: `mount::start_stop_riding`
//! seats him from the attach in the net state (his locomotion goes), `mount::pose_riders` puts him on the seat and plays
//! the riding idle, and combat's `SCSkillFired` handling plays the slash on his upper body (`Stage::play` on a seated
//! rider, `docs/aa-client-combat.md`, "Mounted casters"). The skill fires every second from 2.6 s, so a shot at any time
//! catches a slash. `AA_RIDER_ANIM=394` fires Mounted Defense's 394 instead (its `ride_ub`, `onehand_co_sk_shielddefense_mub`).
//!
//! It needs `AA_PAK` and `AA_DB`, and exits non-zero when the rider was not seated or the clip did not play as a layer:
//!
//! `cargo run -p aa-client --example rider_attack -- --mute --shot shots/aa-client/rider_attack.png --shot-after 6`
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
#[path = "../src/priority.rs"]
mod priority;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/skilltrace/mod.rs"]
mod skilltrace;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;
#[path = "../src/lua_value.rs"]
mod lua_value;
use aa_net::model::{Attachment, Equipment, ModelParams, Unit, UnitKind};
use aa_net::skill::{SkillFired, SkillTarget};
use aa_net::{Event, NetEvent, Pos, SkillCasterData, SkillTargetData};
use bevy::prelude::*;
use ingame::combat::Acting;
use lua_value::lua_int;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    InGame,
    Login,
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    Loading,
}

const HORSE: u32 = 9_000_001;
const RIDER: u32 = 9_000_002;
/// The first fire (s after start) and the time between two (Mounted Attack's clip is 1.2 s long).
const FIRST: f32 = 2.6;
const EVERY: f32 = 1.0;

/// The rider's entity (`mount::Rider` is what the mount systems put on it).
#[derive(Component)]
struct Man;

/// What the rider did: seated without locomotion, and the fired clip seen as an upper body action.
#[derive(Resource, Default)]
struct Seen {
    seated: bool,
    layered: bool,
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Rider attack"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_world::pipelines::build,
        ))
        .init_state::<Screen>()
        .insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 400.0, ..default() })
        .insert_resource(ClearColor(Color::srgb(0.32, 0.4, 0.5)))
        .init_resource::<aa_net::state::Units>()
        .init_resource::<aa_net::state::Doodads>()
        .init_resource::<aa_net::state::Skills>()
        .init_resource::<aa_net::state::Buffs>()
        .init_resource::<ingame::units::Views>()
        .init_resource::<Seen>()
        .add_message::<NetEvent>()
        .add_message::<aa_net::NetCommand>()
        .add_plugins(ingame::combat::plugin)
        .add_systems(Startup, scene)
        .add_systems(Update, (fire, ingame::mount::start_stop_riding, ingame::mount::pose_riders, watch).chain())
        .add_systems(Last, check)
        .run();
}

fn scene(mut commands: Commands, mut cry: aa_cry::scene::CryAssets, clips: Res<aa_anim::ClipLibrary>, mut units: ResMut<aa_net::state::Units>, mut views: ResMut<ingame::units::Views>) {
    let db = ingame::data::Db::open().expect("game database (AA_DB)");
    let game = aa_anim::GameDb::open().expect("game database (AA_DB)");
    let file = db.actor_model_file(139).expect("model 139 (horse) has an actor model file");
    let horse = unit(HORSE, UnitKind::Mate { tl_id: 1, template_id: 0, owner_id: 0 }, 139);
    let mut rider = unit(RIDER, UnitKind::Character { char_id: 42 }, 1);
    rider.attached_to = Some(Attachment { obj_id: HORSE, point: 1, reason: 1 });
    let mount = ingame::creature::spawn(&mut commands, &mut cry, &clips, &file, None, Transform::IDENTITY).unwrap_or_else(|| panic!("{file} cannot be spawned"));
    commands.entity(mount).insert(ingame::units::RemoteUnit::announced(&horse));
    views.track_unit_entity(HORSE, mount);
    let assets = aa_anim::CharacterDesc::default_for(&game, aa_anim::Race::Nuian, aa_anim::Gender::Male).and_then(|d| d.resolve(&game)).expect("a Nuian man");
    let man = aa_anim::spawn_character(&mut commands, &mut cry, &clips, assets, Transform::IDENTITY).expect("the rider");
    commands.entity(man).insert((ingame::units::RemoteUnit::announced(&rider), Man));
    views.track_unit_entity(RIDER, man);
    units.by_id.insert(HORSE, horse);
    units.by_id.insert(RIDER, rider);
    commands.insert_resource(db);
    commands.spawn((
        Mesh3d(cry.meshes.add(Plane3d::default().mesh().size(30.0, 30.0))),
        MeshMaterial3d(cry.materials.add(StandardMaterial { base_color: Color::srgb(0.3, 0.36, 0.24), perceptual_roughness: 1.0, ..default() })),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 12000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    // from the horse's right, a little ahead: the right hand's slash faces the camera
    commands.spawn((Camera3d::default(), Transform::from_xyz(4.5, 2.4, -2.0).looking_at(Vec3::new(0.0, 1.7, 0.0), Vec3::Y)));
}

/// A unit at the origin as `SCUnitState` announces it.
fn unit(obj_id: u32, kind: UnitKind, model_id: u32) -> Unit {
    Unit {
        obj_id,
        name: if obj_id == HORSE { "Horse" } else { "Rider" }.into(),
        kind,
        owner_name: String::new(),
        pos: Pos::new(0.0, 0.0, 0.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 30,
        model_id,
        equipment: Equipment::default(),
        model_params: ModelParams::default(),
        hp: 100,
        mp: 100,
        max_hp: 100,
        max_mp: 100,
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

/// `SCSkillFired` of the rider's skill, every second.
fn fire(time: Res<Time>, mut next: Local<f32>, mut out: MessageWriter<NetEvent>) {
    let now = time.elapsed_secs();
    if *next == 0.0 {
        *next = FIRST;
    }
    if now < *next {
        return;
    }
    *next += EVERY;
    let anim = std::env::var("AA_RIDER_ANIM").ok().and_then(|v| v.parse().ok()).unwrap_or(84);
    let skill = if anim == 394 { 11036 } else { 11013 };
    let raw = SkillCasterData::Unit(RIDER);
    out.write(NetEvent(Event::SkillFired(SkillFired {
        skill_id: skill,
        tl: (now * 10.0) as u16,
        caster: raw.projection(),
        target: SkillTarget::unit(0),
        delay_ms: 0,
        channeling_ms: 0,
        fire_anim_id: anim,
        raw_final_flag: 0,
        result: Default::default(),
        raw_caster: raw,
        raw_target: SkillTargetData::Unit(0),
    })));
}

fn watch(
    mut seen: ResMut<Seen>,
    rider: Query<(Has<ingame::mount::Rider>, Has<ingame::anim::Locomotion>, Option<&Acting>, &aa_anim::Animator), With<Man>>,
) {
    let Ok((seated, walking, acting, animator)) = rider.single() else { return };
    if seated && !walking && !seen.seated {
        info!("rider_attack: seated");
        seen.seated = true;
    }
    if acting.is_some_and(|a| a.layered) && animator.has_layer() && !seen.layered {
        let base = animator.current.as_ref().and_then(|p| ingame::anim::clip_file(&p.clip));
        info!("rider_attack: the skill plays on the upper body over {}", base.as_deref().unwrap_or("-"));
        seen.layered = true;
    }
}

/// By 5.5 s the rider sits and has played the clip as a layer.
fn check(time: Res<Time>, seen: Res<Seen>, mut checked: Local<bool>) {
    if *checked || time.elapsed_secs() < 5.5 {
        return;
    }
    *checked = true;
    if !seen.seated || !seen.layered {
        error!("rider_attack: seated {}, upper body action {}", seen.seated, seen.layered);
        std::process::exit(1);
    }
}
