//! Original instrument/helper and native-derived base seating control; synthetic receipts, no socket.
#![allow(dead_code)]
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
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

use aa_net::model::{CharacterState, Doodad, Unit, UnitKind};
use aa_net::{Event, NetCommand, NetEvent, Pos};

#[derive(Resource)]
struct Fixture {
    before: bool,
    exit_after: Option<f32>,
    input_sent: bool,
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let exit_after = args
        .windows(2)
        .find(|w| w[0] == "--unbond-after")
        .map(|w| w[1].parse().unwrap());
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Original doodad seating"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_anim::facial::FacialPlugin,
            aa_world::pipelines::build,
        ))
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(Fixture {
            before: args.iter().any(|a| a == "--before"),
            exit_after,
            input_sent: false,
        })
        .insert_resource(ingame::data::Db::open().expect("read-only game catalog"))
        .init_resource::<ingame::units::Views>()
        .init_resource::<aa_net::state::Units>()
        .init_resource::<aa_net::state::Doodads>()
        .init_resource::<ingame::player::PlayerInput>()
        .init_resource::<ingame::teleport::Teleport>()
        .add_message::<NetCommand>()
        .add_message::<NetEvent>()
        .add_plugins((ingame::doodad_visual::plugin, ingame::seating::plugin))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 200.0,
            ..default()
        })
        .add_systems(Startup, (stage, scene))
        .add_systems(
            Update,
            (
                ingame::units::sync_views,
                fixture_input.before(ingame::seating::track),
                report.after(ingame::camera::follow_player),
            ),
        )
        .run();
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let side = std::env::args().any(|a| a == "--side");
    let eye = if side {
        Vec3::new(4.0, 2.3, 1.3)
    } else {
        Vec3::new(-3.1, 2.7, 4.5)
    };
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(eye).looking_at(Vec3::new(0.0, 0.85, 0.6), Vec3::Y),
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
        Mesh3d(meshes.add(Plane3d::default().mesh().size(12.0, 12.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.16, 0.19, 0.21))),
    ));
}

fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    mut out: MessageWriter<NetEvent>,
    mut doodads: ResMut<aa_net::state::Doodads>,
    fixture: Res<Fixture>,
) {
    use aa_anim::{
        CharacterDesc,
        db::{GameDb, Gender, Race},
    };
    let args: Vec<_> = std::env::args().collect();
    let drum = args.iter().any(|a| a == "--drum");
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
    state.character.id = 42;
    let unit = Unit {
        obj_id: 47,
        name: "Seating control".into(),
        kind: UnitKind::Character { char_id: 42 },
        owner_name: String::new(),
        pos: Pos::new(0.0, -2.2, 0.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 30,
        model_id: assets.model_id,
        equipment: Default::default(),
        model_params: Default::default(),
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
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
    };
    let placed = Transform::from_xyz(0.0, 0.0, 2.2);
    let actor = ingame::player::spawn_with_locomotion(
        &mut commands,
        &mut cry,
        &clips,
        assets,
        placed,
        Vec::new(),
    )
    .unwrap();
    commands.entity(actor).insert((
        ingame::player::LocalPlayer { obj_id: 47 },
        ingame::player::Mover {
            pos: Vec3::new(0.0, -2.2, 0.0),
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
    out.write(NetEvent(Event::CharacterSelected(Box::new(state.clone()))));
    out.write(NetEvent(Event::PlayerSpawn(Box::new(
        aa_net::PlayerSpawn {
            obj_id: 47,
            state,
            unit,
        },
    ))));
    let doodad = Doodad {
        obj_id: 123,
        template_id: if drum { 8065 } else { 1236 },
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 0,
        pos: Pos::default(),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: if drum { 22477 } else { 1951 },
        owner_id: 0,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left: 0,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: 0,
        owner_type: 0,
        owner_db_id: 0,
        data: 0,
    };
    let event = Event::DoodadsSpawned(vec![doodad]);
    doodads.apply(&event);
    out.write(NetEvent(event));
    if !fixture.before {
        out.write(NetEvent(Event::DoodadBond(
            aa_net::DoodadBondEvent::Bonded {
                unit_obj_id: 47,
                bond: aa_net::DoodadBond {
                    owner_obj_id: 123,
                    attach_point: 2,
                    kind: 2,
                    space: 1,
                    spot: 0,
                },
            },
        )));
    }
}

fn fixture_input(
    time: Res<Time>,
    mut fixture: ResMut<Fixture>,
    mut input: ResMut<ingame::player::PlayerInput>,
    mut commands: MessageReader<NetCommand>,
    mut out: MessageWriter<NetEvent>,
) {
    input.moving =
        fixture.exit_after.is_some_and(|t| time.elapsed_secs() >= t) && !fixture.input_sent;
    if input.moving {
        fixture.input_sent = true;
    }
    for NetCommand(command) in commands.read() {
        if let aa_net::Command::UnbondDoodad(request) = command {
            // Explicit synthetic send-completion fixture. No socket or fake live-server proof.
            info!("seat-control: synthetic UnbondRequested {:?}", request);
            out.write(NetEvent(Event::DoodadBond(
                aa_net::DoodadBondEvent::UnbondRequested(*request),
            )));
        }
    }
}

fn report(
    time: Res<Time>,
    fixture: Res<Fixture>,
    status: Res<aa_world::pipelines::PipelineStatus>,
    actor: Query<
        (&Transform, &aa_anim::Animator, Has<ingame::seating::Seated>),
        With<ingame::player::LocalPlayer>,
    >,
    doodads: Query<&ingame::doodad_visual::DoodadVisual>,
    mut last: Local<f32>,
) {
    if time.elapsed_secs() - *last < 2.0 {
        return;
    }
    *last = time.elapsed_secs();
    if let Ok((transform, animator, seated)) = actor.single() {
        info!(
            "seat-control: before={} seated={} position={:?} scale={:?} loop={} doodads={} pipelines={} waiting={}",
            fixture.before,
            seated,
            transform.translation,
            transform.scale,
            animator.current.as_ref().is_some_and(|p| p.looping),
            doodads.iter().count(),
            status.total(),
            status.waiting()
        );
    }
}
