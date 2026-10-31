//! A grown crop the player planted offers Harvest on the dynamic action bar, through the game's own HUD; no server.
//!
//! Rice Plant 2246 phase 4404 has one `DoodadFuncUse`: `func_skill_id` 13980 Harvest (`skills.max_range` 4, 1 LP,
//! 4000 ms). The example stages a character that owns the crop 3 m in front of it (the original `SCDoodadCreated`
//! owner fields: `owner_type` CHARACTER 254, `owner_id` the character), lets `units::sync_views` draw it and the
//! original Lua dynamic action bar read it, then executes the bar's first entry. The bar and the `UseDoodadSkill`
//! command it produces are checked; without a server the command is not sent. It then moves the crop to 4.5 m, just
//! beyond the 4 m reach, and checks that the bar offers nothing there.
//!
//! F15 (the owner-planted crop's bar) was not driven in the 2026-10-30 original-client capture (`GetDynamicActionCount`
//! was nil), so the original's bar entries, reach and labor for a planted crop remain capture-pending. This example
//! checks the client's generic slice-1 path against the database: phase 4404 -> Harvest 13980 in range.
//!
//! Needs `AA_PAK` and `AA_DB`; without them it says so and exits 0.
//!
//! `cargo run -p aa-client --example farm_harvest_bar -- --mute --offscreen-shot shots/aa-client/farm_harvest_bar.png --shot-after 8`
//! takes the picture at 8 s; the checks run at 7 s. Without a shot the run ends after the checks.
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
use aa_net::{Command, Event, NetCommand, NetEvent, Pos};
use aa_world::cry_to_bevy;

/// The player's object and character ids (the crop's owner).
const PLAYER_OBJ: u32 = 47;
const PLAYER_CHARACTER: u32 = 42;
const PLAYER_NAME: &str = "Planter";
const FULL: i32 = 50_000;
/// Above the Harvest skill's 1 LP, so the HUD's labor gate lets the use through.
const LABOR: i16 = 500;
const PLAYER_AT: Vec3 = Vec3::ZERO;
/// Rice Plant 2246, grown phase 4404 (Harvest 13980), 3 m north: inside the skill's 4 m reach.
const RICE_TEMPLATE: u32 = 2_246;
const RICE_PHASE: u32 = 4_404;
const RICE_OBJ: u32 = 3_101;
const RICE_AT: Vec3 = Vec3::new(0.0, 3.0, 0.0);
/// Just beyond the Harvest skill's 4 m reach: the bar must offer nothing here.
const RICE_OUT_AT: Vec3 = Vec3::new(0.0, 4.5, 0.0);
const HARVEST_SKILL: u32 = 13_980;
/// The bar is read at this second and its first entry executed.
const EXEC_AT: f32 = 3.0;
/// The crop is moved out of reach here, and the bar read back a moment later.
const OUT_AT: f32 = 5.0;
const OUT_READ_AT: f32 = 5.5;
/// The checks run here.
const CHECK_AT: f32 = 7.0;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!(
            "farm_harvest_bar: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)"
        );
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "farm_harvest_bar: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("farm_harvest_bar: skipped, the game database cannot be opened (AA_DB)");
        return AppExit::Success;
    };
    if shot_mode() && shot_after() <= CHECK_AT {
        eprintln!(
            "farm_harvest_bar: --shot-after {} is not after the check at {CHECK_AT} s; raise it",
            shot_after()
        );
        return AppExit::error();
    }
    App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("aa-client: farm harvest bar"),
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
        // what the HUD's systems read and `ingame::InGamePlugin` provides (it also loads the world, which this does not)
        .init_resource::<ingame::player::PlayerBuilt>()
        .init_resource::<ingame::MountReason>()
        .add_message::<ingame::housing::BuilderSignal>()
        .init_resource::<ingame::housing::placement::Builder>()
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .init_resource::<BarCheck>()
        .add_plugins((
            ingame::ground_target::plugin,
            hud::HudPlugin,
            ingame::doodad_visual::plugin,
            // the HUD's doodad tracking writes the use-target decal this resource belongs to
            ingame::doodad_affordance::plugin,
        ))
        .add_systems(Startup, (stage, scene))
        .add_systems(
            Update,
            (
                ingame::units::sync_views.run_if(ingame::world_screens),
                watch_commands,
                (move_crop, drive_bar, read_out, report).chain(),
            ),
        )
        .run()
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        bevy::camera::Hdr,
        Msaa::Off,
        Transform::from_xyz(9.0, 5.0, 4.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
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

/// The grown rice the player planted, announced the way `SCDoodadCreated` reports it.
fn doodad(at: Vec3, owner: u32) -> Doodad {
    Doodad {
        obj_id: RICE_OBJ,
        template_id: RICE_TEMPLATE,
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 255,
        pos: Pos::new(at.x, at.y, at.z),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: RICE_PHASE,
        owner_id: owner,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left: 0,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: -1,
        owner_type: 254,
        owner_db_id: 0,
        data: 0,
    }
}

/// The player (the real character model, idling) and the crop, announced the way the session reports them.
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
    state.character.labor_power = LABOR;
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
    let announced = Event::DoodadsSpawned(vec![doodad(RICE_AT, PLAYER_CHARACTER)]);
    doodads.apply(&announced);
    out.write(NetEvent(announced));
}

/// The bar as it was when it was executed, and the skill command it produced.
#[derive(Resource, Default)]
struct BarCheck {
    executed: bool,
    bar: Option<String>,
    command: Option<(u32, u32)>,
    out_checked: bool,
    out_bar: Option<String>,
}

/// Records the `UseDoodadSkill` the HUD sends (a live session's `NetPlugin` would put it on the wire).
fn watch_commands(mut commands: MessageReader<NetCommand>, mut check: ResMut<BarCheck>) {
    for NetCommand(command) in commands.read() {
        if let Command::UseDoodadSkill {
            skill_id,
            doodad_obj_id,
        } = command
        {
            check.command = Some((*skill_id, *doodad_obj_id));
        }
    }
}

/// Moves the crop to just beyond the skill's reach at [`OUT_AT`]. `find_focus` reads the doodads' state positions, so the
/// bar drops it even though the staged model stays where it was drawn.
fn move_crop(time: Res<Time>, mut doodads: ResMut<aa_net::state::Doodads>, mut moved: Local<bool>) {
    if *moved || time.elapsed_secs() < OUT_AT {
        return;
    }
    *moved = true;
    if let Some(doodad) = doodads.by_id.get_mut(&RICE_OBJ) {
        doodad.pos = Pos::new(RICE_OUT_AT.x, RICE_OUT_AT.y, RICE_OUT_AT.z);
        info!(
            "farm-harvest-bar: crop {RICE_OBJ} moved to {:.1} m, beyond the 4 m reach",
            RICE_OUT_AT.length()
        );
    }
}

/// After [`OUT_READ_AT`]: the bar's visible/count state with the crop out of range.
fn read_out(time: Res<Time>, rt: Option<NonSend<aa_ui::UiRuntime>>, mut check: ResMut<BarCheck>) {
    if check.out_checked || time.elapsed_secs() < OUT_READ_AT {
        return;
    }
    let Some(rt) = rt else { return };
    let bar = rt
        .lua
        .load("return tostring(dynamic_action_bar:IsVisible()) .. ' x' .. tostring(X2:GetDynamicActionCount())")
        .eval::<String>();
    if let Ok(bar) = bar {
        check.out_bar = Some(bar);
        check.out_checked = true;
    }
}

/// At [`EXEC_AT`]: read the bar through the original Lua and execute its first entry.
fn drive_bar(time: Res<Time>, rt: Option<NonSend<aa_ui::UiRuntime>>, mut check: ResMut<BarCheck>) {
    if check.executed || time.elapsed_secs() < EXEC_AT {
        return;
    }
    let Some(rt) = rt else { return };
    let bar = rt
        .lua
        .load("return tostring(dynamic_action_bar:IsVisible()) .. ' x' .. tostring(X2:GetDynamicActionCount()) .. ' ' .. tostring(X2:GetDynamicActionSkill(1))")
        .eval::<String>();
    let Ok(bar) = bar else { return };
    let expected = format!("true x1 {HARVEST_SKILL}");
    if bar != expected {
        // the doodad model may still be loading; keep the last reading and let the check report it
        check.bar = Some(bar);
        if time.elapsed_secs() >= CHECK_AT {
            check.executed = true;
        }
        return;
    }
    check.executed = true;
    check.bar = Some(bar.clone());
    info!("farm-harvest-bar: bar `{bar}` at 3 m from the grown rice");
    let _ = rt.lua.load("X2:ExecuteDynamicAction(1)").exec();
}

/// At [`CHECK_AT`]: the grown crop showed Harvest in range and the bar's use reached the net command.
fn report(
    time: Res<Time>,
    check: Res<BarCheck>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<bool>,
) {
    if *done || time.elapsed_secs() < CHECK_AT {
        return;
    }
    *done = true;
    let mut failures = Vec::new();
    let bar = check.bar.clone().unwrap_or_default();
    if bar != format!("true x1 {HARVEST_SKILL}") {
        failures.push(format!("the bar was `{bar}`, not Harvest {HARVEST_SKILL}"));
    }
    if check.command != Some((HARVEST_SKILL, RICE_OBJ)) {
        failures.push(format!(
            "the bar sent {:?}, not UseDoodadSkill {HARVEST_SKILL} on {RICE_OBJ}",
            check.command
        ));
    }
    let out = check.out_bar.clone().unwrap_or_default();
    if out != "false x0" {
        failures.push(format!(
            "the bar was `{out}` at {:.1} m (4 m reach), not hidden with no entry",
            RICE_OUT_AT.length()
        ));
    }
    if failures.is_empty() {
        info!("farm-harvest-bar: PASSED");
        if !shot_mode() {
            exit.write(AppExit::Success);
        }
    } else {
        error!("farm-harvest-bar: FAILED: {}", failures.join("; "));
        exit.write(AppExit::error());
    }
}

fn shot_mode() -> bool {
    aa_dev::args()
        .iter()
        .any(|a| a == "--shot" || a == "--offscreen-shot")
}

fn shot_after() -> f32 {
    let args = aa_dev::args();
    args.iter()
        .position(|a| a == "--shot-after")
        .and_then(|i| args.get(i + 1))
        .and_then(|t| t.parse().ok())
        .unwrap_or(5.0)
}
