//! What a doodad offers the player: the use-target ring under a decal portal the dynamic action bar is about, and the
//! hover plate of a planted tree with its growth timer, through the systems the game runs; synthetic receipts, no socket.
//!
//! A Halcyona Worldgate (template 4608: the four continent "space gap" portals are the only doodads with
//! `use_target_decal`; its one func is a `DoodadFuncFakeUse` whose skill 17557 "Enter Continental Worldgate" reaches
//! 4 m) stands 3.2 m in front of the player: the original `dynamic_action_bar` offers the skill (key F) and
//! `ingame::doodad_affordance` draws the ring under the gate. A Fig Tree the player planted (template 380, phase 1779:
//! `DoodadFuncGrowth` over 514 s, an Uproot use, 8 min left) stands 2.8 m behind: the bar prefers the doodad in front
//! (`hud::doodad::find_focus`) and `--doodad-hover T` puts the pointer on the nearest usable doodad, the tree, so the
//! original `interaction` script's plate shows its name, its owner and the growth time left.
//!
//! The game's own systems run: `hud::HudPlugin` (the stage built from the original Lua UI, `hud::doodad`),
//! `ingame::units::sync_views` (the doodad models), `ingame::doodad_visual` (phase, growth timer, glow) and
//! `ingame::doodad_affordance` (the ring). Staged by hand, as in `doodad_seating`: the floor, the sun, the camera, and
//! the player and the doodads, announced by `aa_net` events applied to the state. The world is not loaded, so the ring
//! drapes over a flat floor.
//!
//! `cargo run -p aa-client --example doodad_affordance -- --mute --doodad-hover 4 --offscreen-shot shots/aa-client/doodad_affordance.png --shot-after 12`
//! (`--shot` instead of `--offscreen-shot` draws into a window; `--headless-fps 1000` lets the offscreen loop run
//! uncapped, so the logged `aa-dev: fps=` is the renderer's). Needs `AA_PAK` and `AA_DB`; without them it says so and
//! ends. `AA_SETTINGS_FILE=<file>` keeps the run away from the account's saved settings.
#![allow(dead_code)]
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/lua_value.rs"]
mod lua_value;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;
use bevy::prelude::*;
use lua_value::lua_int;
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

use aa_net::model::{CharacterState, Doodad, Unit, UnitKind};
use aa_net::{Event, NetEvent, Pos};
use aa_world::cry_to_bevy;

/// The player's object and character ids (the character owns the tree).
const PLAYER_OBJ: u32 = 47;
const PLAYER_CHARACTER: u32 = 42;
const PLAYER_NAME: &str = "Planter";
/// Above the maximum the HUD computes for a level 30 character: a full bar, no low health warning.
const FULL: i32 = 50_000;
/// Game coordinates (x east, y north, z up): the player stands here, facing north.
const PLAYER_AT: Vec3 = Vec3::ZERO;
/// The Halcyona Worldgate, 3.2 m ahead: inside the 4 m reach of its skill.
const PORTAL: (u32, u32, u32) = (4608, 11368, 3_001);
const PORTAL_AT: Vec3 = Vec3::new(0.0, 3.2, 0.0);
/// The turn about the vertical (radians): the camera looks through the gate.
const PORTAL_TURN: f32 = std::f32::consts::FRAC_PI_2;
/// The Fig Tree the player planted (phase 1779: growth 0.8 -> 1.0 over 514 s), 2.8 m behind.
const TREE: (u32, u32, u32) = (380, 1779, 3_002);
const TREE_AT: Vec3 = Vec3::new(0.0, -2.8, 0.0);
/// The growth left when the tree is announced, milliseconds (8 min).
const TREE_TIME_LEFT: u32 = 480_000;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("doodad_affordance: skipped, no game_pak (set AA_PAK)");
        return AppExit::Success;
    }
    if !aa_anim::db::GameDb::available() {
        eprintln!("doodad_affordance: skipped, no game database (set AA_DB)");
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("doodad_affordance: skipped, the game database could not be opened (AA_DB)");
        return AppExit::Success;
    };
    if hover_from().is_none() {
        eprintln!("doodad_affordance: pass --doodad-hover T to hover the tree (its plate)");
        if shot_mode() {
            // the unattended check needs the plate: without the hover it could only pass by not checking it
            return AppExit::error();
        }
    }
    if shot_mode() && shot_after() <= check_at() {
        // the shot ends the run: taken before the check, the run would pass without checking anything
        eprintln!(
            "doodad_affordance: --shot-after {} is not after the check at {} s (hover start + 2 s); raise it",
            shot_after(),
            check_at()
        );
        return AppExit::error();
    }
    App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("aa-client: doodad affordance"),
            aa_dev::DevPlugin,
            // the state resources; its session thread idles, nothing logs in
            aa_net::NetPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_anim::facial::FacialPlugin,
            aa_world::pipelines::build,
        ))
        .insert_state(Screen::InGame)
        .insert_resource(ClearColor(Color::srgb(0.36, 0.5, 0.68)))
        .insert_resource(db)
        .init_resource::<ingame::units::Views>()
        .init_resource::<ingame::DevOptions>()
        // what the HUD's systems read and `ingame::InGamePlugin` provides (it also loads the world, which this does not):
        // the player's build flag, the mount reason and the housing builder; ground targeting is added below
        .init_resource::<ingame::player::PlayerBuilt>()
        .init_resource::<ingame::MountReason>()
        .add_message::<ingame::housing::BuilderSignal>()
        .init_resource::<ingame::housing::placement::Builder>()
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .add_plugins((
            ingame::ground_target::plugin,
            hud::HudPlugin,
            ingame::doodad_visual::plugin,
            ingame::doodad_affordance::plugin,
        ))
        .add_systems(Startup, (stage, scene))
        .add_systems(
            Update,
            (
                ingame::units::sync_views.run_if(ingame::world_screens),
                report,
            ),
        )
        .run()
}

/// The camera, the light and the floor (as the other doodad examples). The camera is an HDR one without MSAA like the
/// world's (`hud::cameras::match_world_camera` makes the UI cameras agree) and looks at the player from the east: the
/// worldgate is on the right, the tree on the left.
fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        bevy::camera::Hdr,
        Msaa::Off,
        Transform::from_xyz(12.0, 6.0, 0.9).looking_at(Vec3::new(0.0, 2.2, -0.3), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.22, 0.27, 0.17))),
    ));
}

/// The doodad (template, phase `doodad_func_groups.id`, object id) as `SCDoodadsCreated` announces it: unparented, turned
/// `turn` radians about the vertical, owned by the character `owner` (0: nobody) and with `time_left` milliseconds of its
/// phase to go.
fn doodad(
    (template_id, func_group_id, obj_id): (u32, u32, u32),
    at: Vec3,
    turn: f32,
    owner: u32,
    time_left: u32,
) -> Doodad {
    Doodad {
        obj_id,
        template_id,
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 255,
        pos: Pos::new(at.x, at.y, at.z),
        rotation: [0.0, 0.0, (turn / 2.0).sin(), (turn / 2.0).cos()],
        scale: 1.0,
        has_loot_item: false,
        func_group_id,
        owner_id: owner,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: -1,
        owner_type: 255,
        owner_db_id: 0,
        data: 0,
    }
}

/// The player (the real character model, idling) and the two doodads, announced the way the session reports them.
fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    mut out: MessageWriter<NetEvent>,
    mut player: ResMut<aa_net::state::Player>,
    mut doodads: ResMut<aa_net::state::Doodads>,
) {
    use aa_anim::{
        CharacterDesc,
        db::{GameDb, Gender, Race},
    };
    let db = GameDb::open().unwrap();
    let assets = CharacterDesc::from_preset(&db, Race::Nuian, Gender::Male, 0)
        .unwrap()
        .resolve(&db)
        .unwrap();
    let cal = aa_anim::Cal::shared(&assets.cal).unwrap();
    let rig = aa_anim::pose::shared_rig(&assets.skeleton).unwrap();
    let entry = cal.animation("fist_ba_relaxed_idle").unwrap();
    let idle = clips
        .bound(
            &rig,
            &entry.path,
            aa_anim::pose::SampleOptions {
                position_scale: entry.position_scale(cal.base_height),
                lock_root_motion: true,
            },
        )
        .unwrap();
    let mut state = CharacterState::default();
    state.character.id = PLAYER_CHARACTER;
    state.character.name = PLAYER_NAME.into();
    state.character.level = 30;
    state.character.hp = FULL;
    state.character.mp = FULL;
    let unit = Unit {
        obj_id: PLAYER_OBJ,
        name: PLAYER_NAME.into(),
        kind: UnitKind::Character {
            char_id: PLAYER_CHARACTER,
        },
        owner_name: String::new(),
        pos: Pos::new(PLAYER_AT.x, PLAYER_AT.y, PLAYER_AT.z),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 30,
        model_id: assets.model_id,
        equipment: Default::default(),
        model_params: Default::default(),
        hp: FULL,
        mp: FULL,
        max_hp: FULL,
        max_mp: FULL,
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
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
    };
    let actor = ingame::player::spawn_with_locomotion(
        &mut commands,
        &mut cry,
        &clips,
        assets,
        Transform::from_translation(cry_to_bevy(PLAYER_AT)),
        Vec::new(),
    )
    .unwrap();
    // `hud::api::find_player` finds the entity by this name, `units::sync_views` by `LocalPlayer`
    commands.entity(actor).insert((
        Name::new(format!("player {PLAYER_NAME}")),
        ingame::player::LocalPlayer { obj_id: PLAYER_OBJ },
        ingame::player::Mover {
            pos: PLAYER_AT,
            facing: 0.0,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
    ));
    commands.queue(move |world: &mut World| {
        let mut animator = world.get_mut::<aa_anim::Animator>(actor).unwrap();
        animator.play(idle, true, 0.0);
        animator.random_idle = None;
    });
    let spawned = Event::PlayerSpawn(Box::new(aa_net::PlayerSpawn {
        obj_id: PLAYER_OBJ,
        state,
        unit,
    }));
    player.apply(&spawned);
    out.write(NetEvent(spawned));
    let announced = Event::DoodadsSpawned(vec![
        doodad(PORTAL, PORTAL_AT, PORTAL_TURN, 0, 0),
        doodad(TREE, TREE_AT, 0.0, PLAYER_CHARACTER, TREE_TIME_LEFT),
    ]);
    doodads.apply(&announced);
    out.write(NetEvent(announced));
}

/// When the run is unattended (`--shot` / `--offscreen-shot`) and must fail on its own.
fn shot_mode() -> bool {
    aa_dev::args().iter().any(|a| a == "--shot" || a == "--offscreen-shot")
}

/// `--doodad-hover T`: the second from which the pointer is on the tree (parsed by `hud::doodad` as well).
fn hover_from() -> Option<f32> {
    let args = aa_dev::args();
    let i = args.iter().position(|a| a == "--doodad-hover")?;
    args.get(i + 1).and_then(|t| t.parse().ok())
}

/// `--shot-after` as `aa_dev` reads it (default 5 s).
fn shot_after() -> f32 {
    let args = aa_dev::args();
    args.iter()
        .position(|a| a == "--shot-after")
        .and_then(|i| args.get(i + 1))
        .and_then(|t| t.parse().ok())
        .unwrap_or(5.0)
}

/// The scene is staged and the HUD has settled by then.
const CHECK_AFTER_SECS: f32 = 4.0;

/// When the unattended check runs: after [`CHECK_AFTER_SECS`] and 2 s after the hover starts (the plate needs it).
fn check_at() -> f32 {
    CHECK_AFTER_SECS.max(hover_from().unwrap_or(0.0) + 2.0)
}

/// Every 2 s: what the slice's systems decided (the ring's doodad, the glowing ones) and what the original scripts show
/// (the dynamic bar and the doodad plate). Unattended, the first report after [`CHECK_AFTER_SECS`] (and 2 s after the
/// hover starts) also checks them and
/// fails the run when the ring is not under the gate, the bar does not offer the gate's one skill or the plate is hidden.
#[allow(clippy::too_many_arguments)]
fn report(
    time: Res<Time>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    ring: Res<ingame::doodad_affordance::DoodadTargetDecal>,
    glow: Res<ingame::doodad_visual::DoodadHighlights>,
    status: Res<aa_world::pipelines::PipelineStatus>,
    mut exit: MessageWriter<AppExit>,
    mut last: Local<f32>,
    mut checked: Local<bool>,
) {
    if time.elapsed_secs() - *last < 2.0 {
        return;
    }
    *last = time.elapsed_secs();
    let lua = |code: &str| {
        rt.as_ref()
            .and_then(|rt| rt.lua.load(code).eval::<String>().ok())
            .unwrap_or_else(|| "-".into())
    };
    let ring_obj = ring.0.map(|r| r.obj_id);
    let bar = lua("return tostring(dynamic_action_bar:IsVisible()) .. ' x' .. tostring(X2:GetDynamicActionCount())");
    let plate = lua("return tostring(fixedTooltip:IsVisible())");
    info!(
        "doodad-affordance: ring={ring_obj:?} glow={:?} bar={bar} plate={plate} pipelines={} waiting={}",
        glow.0,
        status.total(),
        status.waiting()
    );
    if *checked || !shot_mode() || time.elapsed_secs() < check_at() {
        return;
    }
    *checked = true;
    let mut failures = Vec::new();
    if ring_obj != Some(PORTAL.2) {
        failures.push(format!("the ring is under {ring_obj:?}, not the worldgate {}", PORTAL.2));
    }
    if bar != "true x1" {
        failures.push(format!("the dynamic bar is `{bar}`, not visible with the gate's one skill"));
    }
    if plate != "true" {
        failures.push(format!("the tree's plate is `{plate}`, not shown"));
    }
    if !failures.is_empty() {
        error!("doodad-affordance: FAILED: {}", failures.join("; "));
        exit.write(AppExit::error());
    }
}
