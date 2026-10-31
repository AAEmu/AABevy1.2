//! Seed placement through the client's own systems: the rice ghost, the ground ring and the 1.2 refusal texts, with
//! no server and no cursor.
//!
//! A Rice Seed (item 15646, doodad 2246: group 12, `sim_radius` 500 = 0.5 m, start phase 4366) is aimed with the
//! real press route; the example then holds the spot directly (`Aiming::set_plant_preview`) at the distances and
//! neighbours of the 1.2 seed builder rules and checks what `plant::check`/`validate` decided:
//!
//! * 5 m flat is free, 25 m is free (the old 20 m limit would refuse it), 30.4 m and 29 m + 8 m up are
//!   "Too far away." (30 m 3D);
//! * a rice 0.75 m from the spot overlaps (0.75 < 0.5 + 0.5, the old own-radius test allowed it), 0.9 m across and
//!   0.6 m up is free (the old flat test refused it);
//! * no ground under the cursor is "Can only be placed aboveground.";
//! * a second send is not locked (F3: sends 13 s apart), but a teleport/world-load arrival locks the next use for
//!   30 s with "Can't place this for N second(s)." (F3, `FUN_39426520`/`FUN_393a6de0`).
//!
//! Every case is checked after the systems ran; a broken range, overlap, text, ghost, ring colour or reuse lock fails
//! the run.
//! The screenshot is the final free case: the rice seedling model under the cursor and the green `target_correct` ring.
//!
//! `cargo run -p aa-client --example farm_plant_preview -- --mute --offscreen-shot shots/aa-client/farm_plant_preview.png --shot-after 13`
//! (needs `AA_PAK` and `AA_DB`; without them it says so and exits 0). `--shot` draws into a window; `--shot-after` must
//! be after the last check (12.2 s), or the run would end before checking anything. Without a shot flag the run exits
//! 0 after PASS (non-zero after FAIL).
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

use aa_net::model::Doodad;
use aa_net::state::Doodads;
use aa_net::{Command, NetCommand, Pos};
use aa_world::cry_to_bevy;
use ingame::ground_target::{Aiming, GroundSkill, Script, SkillPressed, plant};

/// The rice seed and the doodad it places (1.2 `item_spawn_doodads`; start phase model `common_plants_rice01.cgf`).
const RICE_ITEM: u32 = 15646;
const RICE_DOODAD: u32 = 2246;
/// A group 24 placeable whose non-round shape uses the flat radius in the overlap test.
const SIGN_DOODAD: u32 = 1034;
const PLAYER_OBJ: u32 = 47;
const PLAYER_AT: Vec3 = Vec3::ZERO;
/// The staged neighbour doodad the overlap cases use.
const NEIGHBOUR_OBJ: u32 = 900;
/// Last check time (the lock cases run at 10.6-12.2 s); the screenshot must come after it.
const LAST_CHECK: f32 = 12.2;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("farm_plant_preview: skipped, no game_pak (set AA_PAK)");
        return AppExit::Success;
    }
    if !aa_anim::db::GameDb::available() {
        eprintln!("farm_plant_preview: skipped, no game database (set AA_DB)");
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("farm_plant_preview: skipped, the game database could not be opened (AA_DB)");
        return AppExit::Success;
    };
    if shot_mode() && shot_after() <= LAST_CHECK {
        eprintln!(
            "farm_plant_preview: --shot-after {} is not after the last check at {LAST_CHECK} s; raise it",
            shot_after()
        );
        return AppExit::error();
    }
    let press = SkillPressed {
        skill: 25536,
        item: Some((1, RICE_ITEM)),
        target_obj_id: 0,
        gcd: false,
    };
    let Some((skill, plant)) = plant::begin(&press, &db) else {
        eprintln!("farm_plant_preview: item {RICE_ITEM} places no doodad in this database");
        return AppExit::error();
    };
    App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("aa-client: farm plant preview"),
            aa_dev::DevPlugin,
            aa_net::NetPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_anim::facial::FacialPlugin,
            aa_world::pipelines::build,
        ))
        .insert_state(Screen::InGame)
        .insert_resource(ClearColor(Color::srgb(0.36, 0.5, 0.68)))
        .insert_resource(db)
        .init_resource::<hud::HudPointerOverUi>()
        .insert_resource(Preview {
            press,
            skill,
            plant,
        })
        .insert_resource(CaseState::default())
        .add_plugins(ingame::ground_target::plugin)
        .add_systems(Startup, (stage, player))
        .add_systems(Update, drive)
        .run()
}

/// The aiming of the rice seed, built once with the real [`plant::begin`].
#[derive(Resource)]
struct Preview {
    press: SkillPressed,
    skill: GroundSkill,
    plant: plant::Plant,
}

/// One placement case: the spot (game coordinates, `None`: no ground under the cursor), the neighbour doodad and
/// what the 1.2 rules must decide.
struct Case {
    at: f32,
    spot: Option<Vec3>,
    /// The staged neighbour doodad: its spot and template (group 12 round or group 24 box).
    neighbour: Option<(Vec3, u32)>,
    expect: Option<plant::Refusal>,
    /// The literal 1.2 `ui_texts` line (`SIM_DOODAD_MSG`), not `Refusal::text` (which would compare with itself).
    text: &'static str,
    ring_green: bool,
    why: &'static str,
}

const CASES: [Case; 9] = [
    Case {
        at: 1.0,
        spot: Some(Vec3::new(0.0, 5.0, 0.0)),
        neighbour: None,
        expect: None,
        text: "Can be placed.",
        ring_green: true,
        why: "5 m flat is free",
    },
    Case {
        at: 2.0,
        spot: Some(Vec3::new(0.0, 25.0, 0.0)),
        neighbour: None,
        expect: None,
        text: "Can be placed.",
        ring_green: true,
        why: "25 m is still inside the 30 m",
    },
    Case {
        at: 3.0,
        spot: Some(Vec3::new(0.0, 30.4, 0.0)),
        neighbour: None,
        expect: Some(plant::Refusal::TooFar),
        text: "Too far away.",
        ring_green: false,
        why: "30.4 m is beyond the inclusive 30 m",
    },
    Case {
        at: 4.0,
        spot: Some(Vec3::new(0.0, 29.0, 8.0)),
        neighbour: None,
        expect: Some(plant::Refusal::TooFar),
        text: "Too far away.",
        ring_green: false,
        why: "29 m + 8 m up is 30.08 m",
    },
    Case {
        at: 5.0,
        spot: Some(Vec3::new(0.0, 5.0, 0.0)),
        neighbour: Some((Vec3::new(0.75, 5.0, 0.0), RICE_DOODAD)),
        expect: Some(plant::Refusal::Doodad),
        text: "Too close to other objects.",
        ring_green: false,
        why: "round 0.75 m < 0.5 + 0.5",
    },
    Case {
        at: 6.0,
        spot: Some(Vec3::new(0.0, 5.0, 0.0)),
        neighbour: Some((Vec3::new(0.9, 5.0, 0.6), RICE_DOODAD)),
        expect: None,
        text: "Can be placed.",
        ring_green: true,
        why: "round 0.9 m across and 0.6 m up is 1.08 m in 3D",
    },
    Case {
        at: 7.0,
        spot: Some(Vec3::new(0.0, 5.0, 0.0)),
        neighbour: Some((Vec3::new(0.4, 5.0, 0.0), SIGN_DOODAD)),
        expect: Some(plant::Refusal::Doodad),
        text: "Too close to other objects.",
        ring_green: false,
        why: "box 0.4 m < 0.5 m flat",
    },
    Case {
        at: 8.0,
        spot: Some(Vec3::new(0.0, 5.0, 0.0)),
        neighbour: Some((Vec3::new(0.6, 5.0, 0.0), SIGN_DOODAD)),
        expect: None,
        text: "Can be placed.",
        ring_green: true,
        why: "box 0.6 m > 0.5 m flat",
    },
    Case {
        at: 9.0,
        spot: None,
        neighbour: None,
        expect: Some(plant::Refusal::NoGround),
        text: "Can only be placed aboveground.",
        ring_green: false,
        why: "no ground under the cursor",
    },
];

/// The final free case the screenshot shows.
const LAST_CASE: Case = Case {
    at: 10.0,
    spot: Some(Vec3::new(0.0, 5.0, 0.0)),
    neighbour: None,
    expect: None,
    text: "Can be placed.",
    ring_green: true,
    why: "the rice is free again",
};

#[derive(Resource, Default)]
struct CaseState {
    started: Option<f32>,
    pressed: bool,
    /// The press was seen to start plant aiming with the expected item/template (checked once, after the press).
    press_checked: bool,
    applied: bool,
    next: usize,
    /// `aim_input` sent a `CSCreateDoodad` (read on the frame it exists).
    sent_doodad: bool,
    /// 0: first send + second press, 1: check it is not locked, 2: teleport load, 3: check the lock and text,
    /// 4: final preview and shot.
    lock_step: u8,
    failures: Vec<String>,
    done: bool,
}

/// The camera, the light and a floor (the ghost and the ring only need the injected spot, not a world).
fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        bevy::camera::Hdr,
        Msaa::Off,
        Transform::from_xyz(6.0, 4.0, 1.0)
            .looking_at(cry_to_bevy(Vec3::new(0.0, 5.0, 0.5)), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.22, 0.27, 0.17))),
        Transform::from_translation(cry_to_bevy(PLAYER_AT)),
    ));
}

/// The local player `route`/`track_cursor` need (no character model: the shot is about the placement).
fn player(mut commands: Commands) {
    commands.spawn((
        Name::new("planter"),
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
}

fn doodad_at(pos: Vec3, template_id: u32) -> Doodad {
    Doodad {
        obj_id: NEIGHBOUR_OBJ,
        template_id,
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 255,
        pos: Pos::new(pos.x, pos.y, pos.z),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: 4366,
        owner_id: 0,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left: 0,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: -1,
        owner_type: 255,
        owner_db_id: 0,
        data: 0,
    }
}

fn ring_colour(
    reticles: &Query<
        (&Visibility, &MeshMaterial3d<StandardMaterial>),
        With<ingame::ground_target::Reticle>,
    >,
    materials: &Assets<StandardMaterial>,
) -> Option<bool> {
    let (visibility, material) = reticles.iter().next()?;
    if *visibility == Visibility::Hidden {
        return None;
    }
    let c = materials.get(&material.0)?.base_color.to_srgba();
    Some(c.green > c.red)
}

#[allow(clippy::too_many_arguments)]
fn drive(
    time: Res<Time>,
    mut state: ResMut<CaseState>,
    mut aiming: ResMut<Aiming>,
    mut doodads: ResMut<Doodads>,
    preview: Res<Preview>,
    mut pressed: MessageWriter<SkillPressed>,
    mut loads: MessageWriter<ingame::teleport::WorldLoadFinished>,
    mut lock: ResMut<plant::PlantLock>,
    mut script: ResMut<Script>,
    mut sent: MessageReader<NetCommand>,
    ghosts: Query<&Visibility, With<plant::Ghost>>,
    reticles: Query<
        (&Visibility, &MeshMaterial3d<StandardMaterial>),
        With<ingame::ground_target::Reticle>,
    >,
    materials: Res<Assets<StandardMaterial>>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.done {
        return;
    }
    if sent
        .read()
        .any(|NetCommand(command)| matches!(command, Command::CreateDoodad(_)))
    {
        state.sent_doodad = true;
    }
    let now = time.elapsed_secs();
    let start = *state.started.get_or_insert(now);
    let t = now - start;
    if !state.pressed && t >= 0.5 {
        state.pressed = true;
        pressed.write(preview.press);
        return;
    }
    if !state.pressed {
        return;
    }
    if state.next > CASES.len() {
        lock_cases(
            t,
            now,
            &mut state,
            &mut aiming,
            &preview,
            &mut pressed,
            &mut loads,
            &mut lock,
            &mut script,
            &ghosts,
            &reticles,
            &materials,
            &mut exit,
        );
        return;
    }
    if !state.press_checked {
        if t < 0.8 {
            return;
        }
        state.press_checked = true;
        let mut failures = Vec::new();
        match aiming.plant() {
            None => failures.push("the seed press did not start plant aiming".into()),
            Some(aimed) => {
                let expected_item = preview.press.item.map(|(item_id, _)| item_id);
                if Some(aimed.item_id) != expected_item {
                    failures.push(format!(
                        "the press aimed item {:?}, expected {expected_item:?}",
                        aimed.item_id
                    ));
                }
                if aimed.doodad != RICE_DOODAD {
                    failures.push(format!(
                        "the press aimed doodad {}, expected {RICE_DOODAD}",
                        aimed.doodad
                    ));
                }
                // The cases keep the separately built Plant, so the press's own must be the same one. `blocked` is
                // the live spot state (NoGround while the cursor has no ground), so compare the identity fields.
                let mut press_plant = aimed.clone();
                press_plant.blocked = None;
                if press_plant != preview.plant {
                    failures.push(format!(
                        "the press's plant {aimed:?} differs from the prebuilt {:?}",
                        preview.plant
                    ));
                }
            }
        }
        if !failures.is_empty() {
            state.failures.extend(failures);
            finish(&mut state, &mut exit);
            return;
        }
    }
    let case = CASES.get(state.next).unwrap_or(&LAST_CASE);
    if !state.applied && t >= case.at {
        state.applied = true;
        aiming.set_plant_preview(
            preview.press,
            preview.skill,
            preview.plant.clone(),
            case.spot.map(|spot| (spot, true)),
        );
        doodads.by_id.clear();
        if let Some((at, template_id)) = case.neighbour {
            doodads
                .by_id
                .insert(NEIGHBOUR_OBJ, doodad_at(at, template_id));
        }
        return;
    }
    if state.next < CASES.len() && !state.applied {
        return;
    }
    if t < case.at + 0.4 {
        return;
    }
    // after the systems have run: the rules, the text, the ghost and the ring
    let mut failures = Vec::new();
    let Some(plant) = aiming.plant() else {
        failures.push(format!("{}: no plant is aimed", case.why));
        state.failures.extend(failures);
        finish(&mut state, &mut exit);
        return;
    };
    if plant.blocked != case.expect {
        failures.push(format!(
            "{}: blocked {:?}, expected {:?}",
            case.why, plant.blocked, case.expect
        ));
    }
    if plant.text() != case.text {
        failures.push(format!(
            "{}: text {:?}, expected {:?}",
            case.why,
            plant.text(),
            case.text
        ));
    }
    match case.spot {
        Some(_) => {
            if ghosts.iter().all(|v| *v == Visibility::Hidden) {
                failures.push(format!("{}: the ghost is hidden", case.why));
            }
            match ring_colour(&reticles, &materials) {
                Some(green) if green == case.ring_green => {}
                other => failures.push(format!(
                    "{}: ring {other:?}, expected green={}",
                    case.why, case.ring_green
                )),
            }
        }
        None => {
            if ghosts.iter().any(|v| *v != Visibility::Hidden) {
                failures.push(format!("{}: the ghost is shown without ground", case.why));
            }
            if ring_colour(&reticles, &materials).is_some() {
                failures.push(format!("{}: the ring is shown without ground", case.why));
            }
        }
    }
    if failures.is_empty()
        && state.next == CASES.len()
        && !plant
            .model
            .as_deref()
            .is_some_and(|m| m.ends_with("common_plants_rice01.cgf"))
    {
        failures.push(format!(
            "the ghost model is {:?}, not the rice start phase",
            plant.model
        ));
    }
    state.failures.extend(failures);
    if state.next < CASES.len() {
        state.next += 1;
        state.applied = false;
    } else {
        // the placement cases passed: move on to the reuse-lock cases
        state.next += 1;
        state.applied = false;
    }
}

/// F3/FUN_39426520: two sends shortly after each other stay unlocked; a teleport/world-load arrival locks for 30 s
/// and the next use shows the countdown text. The final free preview is restored for the screenshot.
#[allow(clippy::too_many_arguments)]
fn lock_cases(
    t: f32,
    now: f32,
    state: &mut CaseState,
    aiming: &mut Aiming,
    preview: &Preview,
    pressed: &mut MessageWriter<SkillPressed>,
    loads: &mut MessageWriter<ingame::teleport::WorldLoadFinished>,
    lock: &mut plant::PlantLock,
    script: &mut Script,
    ghosts: &Query<&Visibility, With<plant::Ghost>>,
    reticles: &Query<
        (&Visibility, &MeshMaterial3d<StandardMaterial>),
        With<ingame::ground_target::Reticle>,
    >,
    materials: &Assets<StandardMaterial>,
    exit: &mut MessageWriter<AppExit>,
) {
    match state.lock_step {
        0 if t >= 10.6 => {
            // the final case left the rice aimed at the free spot; a scripted click goes through aim_input
            script.click();
            state.lock_step = 1;
        }
        1 if t >= 10.9 => {
            if !state.sent_doodad {
                state
                    .failures
                    .push("the scripted click did not send CSCreateDoodad".into());
            }
            if lock.refused(f64::from(now)) {
                state
                    .failures
                    .push("a send started the 30 s reuse lock; F3 shows sends 13 s apart".into());
            }
            // the next plant a moment later: F3 shows two sends 13 s apart both succeed
            pressed.write(preview.press);
            state.lock_step = 2;
        }
        2 if t >= 11.4 => {
            if lock.refused(f64::from(now)) {
                state
                    .failures
                    .push("a send started the 30 s reuse lock; F3 shows sends 13 s apart".into());
            }
            if aiming.plant().is_none() {
                state
                    .failures
                    .push("the second plant was not aimed after the first send".into());
            }
            aiming.cancel();
            loads.write(ingame::teleport::WorldLoadFinished);
            state.lock_step = 3;
        }
        3 if t >= 11.8 => {
            if !lock.refused(f64::from(now)) {
                state
                    .failures
                    .push("a teleport/world load did not start the 30 s reuse lock".into());
            }
            pressed.write(preview.press);
            // after the lock checks, show the final free preview again for the screenshot
            aiming.set_plant_preview(
                preview.press,
                preview.skill,
                preview.plant.clone(),
                Some((Vec3::new(0.0, 5.0, 0.0), true)),
            );
            state.lock_step = 4;
        }
        4 if t >= 12.2 => {
            if aiming.plant().is_none() {
                state
                    .failures
                    .push("planting within 30 s of a teleport was not locked".into());
            }
            if !lock.refused(f64::from(now)) {
                state
                    .failures
                    .push("the teleport lock ended before the countdown check".into());
            }
            let secs = lock.remaining(f64::from(now)) as u64;
            if !(28..=30).contains(&secs) {
                state.failures.push(format!(
                    "the teleport countdown is {secs} s, expected about 30"
                ));
            }
            let text = plant::lock_text(secs);
            let want = match secs {
                28 => "Can't place this for 28 second(s).",
                29 => "Can't place this for 29 second(s).",
                30 => "Can't place this for 30 second(s).",
                _ => "",
            };
            if text != want {
                state
                    .failures
                    .push(format!("the countdown text is {text:?}, expected {want:?}"));
            }
            // the restored preview must be the visible rice ghost under a green ring at shot time
            match aiming.plant() {
                Some(plant) if plant.blocked.is_none() && plant.text() == plant::CAN_BE_PLACED => {}
                other => state
                    .failures
                    .push(format!("the final preview is {other:?}")),
            }
            if !ghosts.iter().any(|v| *v != Visibility::Hidden) {
                state
                    .failures
                    .push("the final rice ghost is hidden at shot time".into());
            }
            if ring_colour(reticles, materials) != Some(true) {
                state
                    .failures
                    .push("the final ring is not green at shot time".into());
            }
            finish(state, exit);
        }
        _ => {}
    }
}

fn finish(state: &mut CaseState, exit: &mut MessageWriter<AppExit>) {
    if state.done {
        return;
    }
    state.done = true;
    if state.failures.is_empty() {
        info!("farm-plant-preview: PASS");
        // a shot run must stay alive for the screenshot; a plain run ends on its own
        if !shot_mode() {
            exit.write(AppExit::Success);
        }
        return;
    }
    error!("farm-plant-preview: FAILED: {}", state.failures.join("; "));
    exit.write(AppExit::error());
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
