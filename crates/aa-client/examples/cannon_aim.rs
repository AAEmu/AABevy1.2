//! A self-checking cannon aim: the local gunner's camera aim is clamped to the galleon cannon's
//! `vehicle_models` turret limits, `CSTurretState` is sent at most every 200 ms and only when
//! the aim changes, and a received posture type 8 turns a remote cannon's `bone_yaw` /
//! `bone_pitch`.
//!
//! `cargo run -p aa-client --example cannon_aim -- --mute --shot shots/aa-client/cannon_aim.png --shot-after 3`
//! Fails (exit 1, no screenshot) when any rule is broken; skips cleanly without `AA_PAK` / `AA_DB`.

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
use lua_value::lua_int;

use aa_net::game::ServerMsg;
use aa_net::model::UnitKind;
use aa_net::{Command, Event, NetCommand, NetEvent, Pos, TurretAim, Unit};
use bevy::prelude::*;

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

const CANNON_MODEL: u32 = 117; // models -> vehicle_models 10 (`galleon.cannon`)
const LOCAL: u32 = 0x10_0001;
const REMOTE: u32 = 0x10_0002;

/// What the local gunner's clamped aim must be for the galleon cannon (`vehicle_models` 10).
const YAW_LIMIT: f32 = 22.0 * std::f32::consts::PI / 180.0;
const PITCH_LIMIT: f32 = 35.0 * std::f32::consts::PI / 180.0;

/// The remote posture fed through the real `SCUnitModelPostureChanged` reader.
const REMOTE_AIM: TurretAim = TurretAim {
    pitch: 0.3,
    yaw: -0.4,
};

#[derive(Resource, Default)]
struct Sent(Vec<(f64, f32, f32)>);

fn main() {
    if std::env::var("AA_PAK").is_err() || std::env::var("AA_DB").is_err() {
        eprintln!("cannon_aim: skip (AA_PAK and AA_DB are required)");
        return;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("cannon_aim: skip (the database cannot be opened)");
        return;
    };
    let Some(limits) = db.turret_limits(CANNON_MODEL) else {
        eprintln!("cannon_aim: FAIL: model {CANNON_MODEL} has no complete turret block");
        std::process::exit(1);
    };
    // The real data row, not the code that reads it (vehicle_models 10).
    let degrees = |v: f32| v.to_degrees();
    if (degrees(limits.pitch_min) + 20.0).abs() > 1e-4
        || (degrees(limits.pitch_max) - 35.0).abs() > 1e-4
        || (degrees(limits.yaw_min) + 22.0).abs() > 1e-4
        || (degrees(limits.yaw_max) - 22.0).abs() > 1e-4
        || (degrees(limits.yaw_rate) - 35.0).abs() > 1e-4
        || (degrees(limits.pitch_rate) - 20.0).abs() > 1e-4
    {
        eprintln!(
            "cannon_aim: FAIL: vehicle_models 10 turret block is {:.1}..{:.1} pitch, {:.1}..{:.1} yaw, {:.1}/{:.1} deg/s",
            degrees(limits.pitch_min),
            degrees(limits.pitch_max),
            degrees(limits.yaw_min),
            degrees(limits.yaw_max),
            degrees(limits.yaw_rate),
            degrees(limits.pitch_rate)
        );
        std::process::exit(1);
    }

    App::new()
        .add_plugins((
            aa_dev::default_plugins("Cannon aim"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_world::pipelines::build,
        ))
        .insert_resource(db)
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .init_resource::<aa_net::state::Units>()
        .init_resource::<Sent>()
        .insert_resource(ingame::LocalMount(Some(ingame::LocalSeat {
            mount: LOCAL,
            point: 1,
            drives: true,
            cart: false,
            ship: false,
            turret: true,
            speed: Default::default(),
            lift: 0.0,
        })))
        .add_message::<NetCommand>()
        .add_message::<NetEvent>()
        .add_plugins(ingame::turret::plugin)
        .add_systems(Startup, (stage, scene).chain())
        .add_systems(
            Update,
            (
                drive_camera,
                ingame::turret::local_aim.after(drive_camera),
                collect,
                feed_remote,
                verify,
            )
                .chain(),
        )
        .run();
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        ingame::camera::OrbitCamera {
            yaw: 1.0,
            pitch: 1.2,
            distance: 8.0,
            wanted: 8.0,
        },
        Transform::from_xyz(4.0, 4.5, 12.0).looking_at(Vec3::new(4.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.16, 0.19, 0.21))),
        Transform::from_xyz(4.0, 0.0, 0.0),
    ));
}

fn cannon_unit(obj_id: u32, pos: Pos) -> Unit {
    Unit {
        obj_id,
        name: "Cannon".into(),
        kind: UnitKind::Slave {
            id: 8,
            tl_id: 1,
            template_id: 10,
            summoner_id: 0,
        },
        owner_name: String::new(),
        pos,
        rotation: [0.0; 3],
        scale: 1.0,
        level: 1,
        model_id: CANNON_MODEL,
        equipment: Default::default(),
        model_params: Default::default(),
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
    let look = db
        .vehicle_look(CANNON_MODEL)
        .expect("model 117 is a vehicle");
    let loader = cry.loader();
    for (obj_id, x) in [(LOCAL, 0.0), (REMOTE, 8.0)] {
        let pieces = ingame::vehicle::prepare(&loader, &look);
        assert!(!pieces.is_empty(), "the galleon cannon has parts");
        let transform = ingame::player::placement(Vec3::new(x, 0.0, 0.0), 0.0, 1.0);
        let root = ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, transform);
        let unit = cannon_unit(obj_id, Pos::new(x, 0.0, 0.0));
        commands
            .entity(root)
            .insert(ingame::units::RemoteUnit::announced(&unit));
        units.by_id.insert(obj_id, unit);
    }
}

/// A moving camera until 1 s, then a fixed aim well past both limits: the target must clamp and
/// stop changing, so no packet goes out after the last pending one.
fn drive_camera(time: Res<Time>, mut cameras: Query<&mut ingame::camera::OrbitCamera>) {
    let t = time.elapsed_secs_f64();
    let (yaw, pitch) = if t < 1.0 {
        // a monotonic aim inside the cannon's -22..22 / -20..35 degree limits: every 200 ms
        // sample is a new aim, so the rate and the change-only rule are both visible
        (-0.35 + 0.7 * t as f32, -0.3 + 0.8 * t as f32)
    } else {
        // past both limits: the clamped target stops changing
        (1.0, 1.2)
    };
    for mut camera in &mut cameras {
        camera.yaw = yaw;
        camera.pitch = pitch;
    }
}

fn collect(mut reader: MessageReader<NetCommand>, time: Res<Time>, mut sent: ResMut<Sent>) {
    for NetCommand(command) in reader.read() {
        if let Command::TurretState { pitch, yaw, .. } = command {
            sent.0.push((time.elapsed_secs_f64(), *pitch, *yaw));
        }
    }
}

/// Feeds the remote cannon the server's type-8 body through the real reader (`SCUnitModelPostureChanged`).
fn feed_remote(time: Res<Time>, mut done: Local<bool>, mut events: MessageWriter<NetEvent>) {
    if *done || time.elapsed_secs() < 0.2 {
        return;
    }
    *done = true;
    let mut body = vec![8, 0];
    body.extend_from_slice(&REMOTE_AIM.pitch.to_le_bytes());
    body.extend_from_slice(&REMOTE_AIM.yaw.to_le_bytes());
    let mut frame = vec![0xDD, 0x01, 0, 0];
    frame.extend(0x0103u16.to_le_bytes());
    frame.extend([
        (REMOTE & 0xff) as u8,
        ((REMOTE >> 8) & 0xff) as u8,
        ((REMOTE >> 16) & 0xff) as u8,
    ]);
    frame.extend(body);
    let decoded = ServerMsg::decode(&frame);
    let Ok(ServerMsg::UnitPosture { obj_id, posture }) = decoded else {
        eprintln!("cannon_aim: FAIL: the type-8 posture frame does not decode: {decoded:?}");
        std::process::exit(1);
    };
    if obj_id != REMOTE || posture.kind != 8 || posture.turret != Some(REMOTE_AIM) {
        eprintln!(
            "cannon_aim: FAIL: decoded posture {obj_id} kind {} {:?}",
            posture.kind, posture.turret
        );
        std::process::exit(1);
    }
    events.write(NetEvent(Event::UnitPosture {
        obj_id,
        kind: 8,
        anim_action: 0,
        activate: false,
        turret: posture.turret,
    }));
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn verify(
    time: Res<Time>,
    mut done: Local<bool>,
    sent: Res<Sent>,
    units: Res<aa_net::state::Units>,
    roots: Query<(Entity, &ingame::units::RemoteUnit)>,
    targets: Query<&ingame::turret::TurretTarget>,
    poses: Query<&ingame::turret::TurretPose>,
    children: Query<&Children>,
    animators: Query<&aa_anim::Animator>,
    bones: Query<&Transform>,
) {
    if *done || time.elapsed_secs() < 2.5 {
        return;
    }
    *done = true;
    let fail = |why: String| -> ! {
        eprintln!("cannon_aim: FAIL: {why}");
        std::process::exit(1);
    };

    // The real data row reached the systems: the local target is the camera aim clamped to the
    // galleon cannon's -22..22 yaw and -20..35 pitch.
    let local = roots
        .iter()
        .find(|(_, u)| u.obj_id == LOCAL)
        .map(|(e, _)| e)
        .unwrap_or_else(|| fail("no local cannon root".into()));
    let local_target = targets
        .get(local)
        .map(|t| t.0)
        .unwrap_or_else(|_| fail("no local turret target".into()));
    if (local_target.yaw - YAW_LIMIT).abs() > 1e-4
        || (local_target.pitch - PITCH_LIMIT).abs() > 1e-4
    {
        fail(format!(
            "local aim {} / {} is not the clamped 22 / 35 degrees",
            local_target.yaw.to_degrees(),
            local_target.pitch.to_degrees()
        ));
    }

    // The 200 ms change-only sends.
    if sent.0.len() < 3 {
        fail(format!(
            "only {} CSTurretState sends in 2.5 s",
            sent.0.len()
        ));
    }
    for pair in sent.0.windows(2) {
        if pair[1].0 - pair[0].0 < 0.195 {
            fail(format!(
                "CSTurretState {} ms apart",
                (pair[1].0 - pair[0].0) * 1000.0
            ));
        }
        if pair[0].1 == pair[1].1 && pair[0].2 == pair[1].2 {
            fail(format!("the same aim was sent twice: {:?}", sent.0));
        }
    }
    for (_, pitch, yaw) in &sent.0 {
        if yaw.abs() > YAW_LIMIT + 1e-4
            || *pitch > PITCH_LIMIT + 1e-4
            || *pitch < -20f32.to_radians() - 1e-4
        {
            fail(format!(
                "sent aim {} / {} is outside the model limits",
                yaw.to_degrees(),
                pitch.to_degrees()
            ));
        }
    }
    // The camera stopped at 1 s: the newest aim was sent by the end of its window (~1.2 s), then
    // an unchanged aim must not be sent again.
    if sent.0.iter().any(|(at, _, _)| *at > 1.3) {
        fail("CSTurretState kept going after the aim stopped changing".into());
    }

    // The remote cannon: the decoded posture reached its target, its drawn aim converged and its
    // identity-bind bones hold the model aim (`FUN_390d14b0`: R_z(yaw), R_x(pitch)).
    let remote = roots
        .iter()
        .find(|(_, u)| u.obj_id == REMOTE)
        .map(|(e, _)| e)
        .unwrap_or_else(|| fail("no remote cannon root".into()));
    let remote_target = targets
        .get(remote)
        .map(|t| t.0)
        .unwrap_or_else(|_| fail("no remote turret target".into()));
    if remote_target != REMOTE_AIM {
        fail(format!(
            "remote target {remote_target:?} is not the received posture"
        ));
    }
    let pose = poses
        .get(remote)
        .map(|p| p.0)
        .unwrap_or_else(|_| fail("no remote turret pose".into()));
    if (pose.pitch - REMOTE_AIM.pitch).abs() > 1e-4 || (pose.yaw - REMOTE_AIM.yaw).abs() > 1e-4 {
        fail(format!(
            "remote pose {pose:?} did not converge to {REMOTE_AIM:?}"
        ));
    }
    let (yaw_bone, pitch_bone) = remote_bones(remote, &children, &animators)
        .unwrap_or_else(|| fail("no bone_yaw / bone_pitch on the remote cannon".into()));
    let yaw = bones
        .get(yaw_bone)
        .unwrap_or_else(|_| fail("no bone_yaw transform".into()))
        .rotation;
    let pitch = bones
        .get(pitch_bone)
        .unwrap_or_else(|_| fail("no bone_pitch transform".into()))
        .rotation;
    let want_yaw = Quat::from_rotation_z(REMOTE_AIM.yaw);
    let want_pitch = Quat::from_rotation_x(REMOTE_AIM.pitch);
    if (yaw * want_yaw.inverse()).angle_between(Quat::IDENTITY) > 1e-4 {
        fail(format!("bone_yaw {yaw:?} is not R_z(-0.4)"));
    }
    if (pitch * want_pitch.inverse()).angle_between(Quat::IDENTITY) > 1e-4 {
        fail(format!("bone_pitch {pitch:?} is not R_x(0.3)"));
    }

    // The units keep their model: a check that this is the real catalog row, not a stand-in.
    if units.by_id.get(&LOCAL).map(|u| u.model_id) != Some(CANNON_MODEL) {
        fail("the local cannon lost its model".into());
    }
    info!(
        "cannon_aim: PASS: {} sends, local {:.1}/{:.1} deg, remote bone_yaw {:.2} bone_pitch {:.2}, {} units",
        sent.0.len(),
        local_target.yaw.to_degrees(),
        local_target.pitch.to_degrees(),
        REMOTE_AIM.yaw,
        REMOTE_AIM.pitch,
        units.by_id.len()
    );
}

fn remote_bones(
    root: Entity,
    children: &Query<&Children>,
    animators: &Query<&aa_anim::Animator>,
) -> Option<(Entity, Entity)> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        let Ok(list) = children.get(entity) else {
            continue;
        };
        stack.extend(list.iter());
        let Ok(animator) = animators.get(entity) else {
            continue;
        };
        let find = |name: &str| {
            animator
                .rig
                .skeleton
                .find_ignore_case(name)
                .and_then(|index| animator.bones.get(index).copied())
        };
        if let (Some(yaw), Some(pitch)) = (find("bone_yaw"), find("bone_pitch")) {
            return Some((yaw, pitch));
        }
    }
    None
}
