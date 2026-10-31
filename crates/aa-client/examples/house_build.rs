//! The house builder and a construction's stage models, from the 2026-10-31 original-client capture; no socket.
//!
//! The client's builder rules are run on the real housing data (`AA_DB` + `AA_PAK`, area 338 of zone 179):
//! the spot Bevy used to accept (15338.53, 14632.33) is refused with `HOUSE_CANNOT_LOCATE_INVALID_AREA`
//! ("You can't construct a building here."), and the captured spot (15328.4, 14631.8, snapped to 15328, 14632)
//! passes. The Start button's `CSCreateHouse` is built through `Builder::commit` and its 45 bytes are compared
//! with the captured row. The stage-0 house comes from the captured `SCMyHouse`/`SCHouseState` body; then
//! `SCHouseBuildProgress` moves it through 1083 -> 1084 -> 1085 -> 1082, and the example checks that the same
//! model keeps its entity (no respawn), that a model change never leaves a frame without a house, and that the
//! final model is 1082.
//!
//! `cargo run -p aa-client --example house_build -- --mute --shot shots/aa-client/house_build.png --shot-after 20`
//! (needs `AA_PAK` and `AA_DB`; without them it says so and exits 0). The run exits non-zero when a check fails.
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

use std::collections::HashMap;

use aa_net::housing::{HouseCommand, HouseSnapshot, HousingEvent};
use aa_net::model::{Unit, UnitKind};
use aa_net::wire::{Pos, Reader};
use aa_net::{Command, Event, Houses, NetCommand, NetEvent};
use ingame::housing::data::{self, HousingData};
use ingame::housing::placement::{self, Build, Builder, BuilderSignal};
use ingame::housing::render::{self, DrawnHouses, HouseModel};
use ingame::player::LocalPlayer;

/// The captured `SCMyHouse`/`SCHouseState` body (2026-10-31, house 13, design 171, model 1083, 0 of 20).
const STAGE_ZERO: &str = "0d00 0d000000 c59800 ab000000 3b040000 01000000 01000000 0500 5265636170 01000000 00 14000000 00000000 f0490200 000000000000be03 0000000000809203 370d2d43 1200 5468617463686564204661726d686f757365 01 00000000 00000000 0000";
/// The captured `CSCreateHouse` body (design 171, 15328/14632, tax 450000, item 16777230).
const CAPTURE_CREATE: [u8; 45] = [
    0xab, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbe, 3, 0, 0, 0, 0, 0, 0x80, 0x92, 3, 0x37, 0x0d, 0x2d, 0x43,
    0, 0, 0, 0, 0x0e, 0, 0, 1, 0, 0, 0, 0, 0xd0, 0xdd, 6, 0, 0, 0, 0, 0, 0,
];
/// The house's object id in the capture.
const HOUSE_OBJ: u32 = 39_109;
/// The builder's player object id does not matter here (the create carries no object id).
const PLAYER_OBJ: u32 = 8615;
/// The captured, client-valid spot and the one the original refuses.
const VALID: (f32, f32) = (15328.4, 14631.8);
const REFUSED: (f32, f32) = (15338.53, 14632.33);
/// Model per construction step (`housing_build_steps` of design 171) and the finished model
/// (`housings.main_model_id`). The 2026-10-31 capture covered action 1 only; actions 2-20, the
/// 10/5/5 boundaries and completion are capture-pending (a second capture is running).
const STAGES: [u32; 4] = [1083, 1084, 1085, 1082];

fn hex(raw: &str) -> Vec<u8> {
    raw.split_whitespace()
        .flat_map(|word| {
            (0..word.len())
                .step_by(2)
                .map(move |i| u8::from_str_radix(&word[i..i + 2], 16).unwrap())
        })
        .collect()
}

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("house_build: skipped, no game_pak (set AA_PAK)");
        return AppExit::Success;
    }
    let Ok(db) = std::env::var("AA_DB") else {
        eprintln!("house_build: skipped, no game database (set AA_DB)");
        return AppExit::Success;
    };
    let housing: HousingData = data::load_now("main_world");
    if !housing.designs.contains_key(&171) || !housing.land.iter().any(|l| l.area_id == 338) {
        eprintln!("house_build: skipped, the housing data is not available");
        return AppExit::Success;
    }
    assert!(data::set(housing), "housing data already loaded");

    let args = aa_dev::DevArgs::from_env();
    let check_at = 20.0;
    if args.shot.is_some() || args.offscreen_shot.is_some() {
        if args.shot_after < check_at {
            eprintln!(
                "house_build: --shot-after {} ends before the check at {check_at} s; raise it",
                args.shot_after
            );
            return AppExit::error();
        }
    } else if args.exit_after.is_none_or(|t| t < check_at) {
        eprintln!("house_build: pass --shot --shot-after {check_at} (or --exit-after {check_at})");
        return AppExit::error();
    }

    App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("aa-client: house build"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_world::pipelines::build,
        ))
        .insert_state(Screen::InGame)
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .init_resource::<aa_net::state::Units>()
        .init_resource::<Houses>()
        .init_resource::<DrawnHouses>()
        .init_resource::<Run>()
        .init_resource::<Texts>()
        .add_message::<NetCommand>()
        .add_message::<NetEvent>()
        .add_message::<BuilderSignal>()
        .init_resource::<Builder>()
        .add_systems(Startup, (stage, house, builder_checks, banner).chain())
        .add_systems(
            Update,
            (verify_commit, drive, render::sync_houses, watch, report).chain(),
        )
        .insert_resource(CheckDb(db))
        .run()
}

#[derive(Resource)]
struct CheckDb(String);

#[derive(Resource, Default)]
struct Run {
    started: Option<f32>,
    stage: usize,
    seen: HashMap<u32, Entity>,
    reported: bool,
    same_model_entity: Option<Entity>,
    same_model_checked: bool,
    failed: Vec<String>,
    done: bool,
}

impl Run {
    fn fail(&mut self, message: String) {
        error!("house_build: {message}");
        self.failed.push(message);
    }
}

#[derive(Resource, Default)]
struct Texts {
    refused: String,
    category: String,
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        bevy::ui::IsDefaultUiCamera,
        Transform::from_xyz(0.0, 9.0, 26.0).looking_at(Vec3::new(0.0, 5.0, 0.0), Vec3::Y),
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
        Mesh3d(meshes.add(Plane3d::default().mesh().size(90.0, 90.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.25, 0.33, 0.18))),
    ));
    commands.spawn((
        LocalPlayer { obj_id: PLAYER_OBJ },
        Transform::from_xyz(0.0, 0.0, 20.0),
    ));
}

fn unit() -> Unit {
    Unit {
        obj_id: HOUSE_OBJ,
        name: "Thatched Farmhouse".into(),
        kind: UnitKind::Housing {
            tl_id: 13,
            template_id: 171,
            build_step: 0,
        },
        owner_name: String::new(),
        pos: Pos::new(0.0, 0.0, 0.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 1,
        model_id: STAGES[0],
        equipment: Default::default(),
        model_params: Default::default(),
        hp: 1,
        mp: 0,
        max_hp: 1,
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
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
    }
}

fn house(
    mut units: ResMut<aa_net::state::Units>,
    mut houses: ResMut<Houses>,
    mut out: MessageWriter<NetEvent>,
) {
    let snapshot =
        HouseSnapshot::read(&mut Reader::new(&hex(STAGE_ZERO))).expect("captured SCHouseState");
    assert_eq!(
        (
            snapshot.tl_id,
            snapshot.obj_id,
            snapshot.model_id,
            snapshot.current_actions
        ),
        (13, HOUSE_OBJ, 1083, 0)
    );
    units.by_id.insert(HOUSE_OBJ, unit());
    houses.apply(&Event::Housing(HousingEvent::State(snapshot.clone())));
    // the unit arrives in the same batch as the state in the capture
    out.write(NetEvent(Event::Housing(HousingEvent::State(snapshot))));
}

/// The real area-338 rules, the captured texts and the Start button's bytes.
fn builder_checks(
    mut net: MessageWriter<NetCommand>,
    mut signals: MessageWriter<BuilderSignal>,
    mut builder: ResMut<Builder>,
    mut texts: ResMut<Texts>,
    mut cry: aa_cry::scene::CryAssets,
    db: Res<CheckDb>,
) {
    let data = data::get().expect("housing data");
    let terrain = aa_world::TerrainQuery::new("main_world");
    // the ghost's finished model box (`FUN_3908d400`), prepared like the ghost's parts
    let parts = render::prepare(&cry.loader(), 1082, &aa_cry::scene::ModelOptions::default());
    let model_box = render::bounds(&parts);
    assert!(model_box.is_some(), "the finished house model has bounds");
    info!("house_build: model box {model_box:?}");
    let site = |pos: (f32, f32)| placement::Site {
        pos: Vec2::new(pos.0, pos.1),
        yaw: 0.0,
        model_box,
    };
    let refused = placement::check(
        data,
        171,
        site(REFUSED),
        placement::Around {
            terrain: Some(&terrain),
            ..default()
        },
    );
    assert_eq!(
        refused,
        Some("HOUSE_CANNOT_LOCATE_INVALID_AREA"),
        "the old spot must be refused"
    );
    let valid = placement::check(
        data,
        171,
        site(VALID),
        placement::Around {
            terrain: Some(&terrain),
            ..default()
        },
    );
    assert_eq!(valid, None, "the captured spot must pass");
    // the captured spot snaps to (15328, 14632)
    assert_eq!(
        placement::snap_spot(Vec2::new(VALID.0, VALID.1), placement::footprint_half(11.0)),
        Vec2::new(15328.0, 14632.0)
    );

    let ui = aa_ui::uitext::load(std::path::Path::new(&db.0), "en_us").expect("ui texts");
    let text = |key: &str| aa_ui::uitext::localize(&ui, 1, key, &[]);
    texts.refused = text("HOUSE_CANNOT_LOCATE_INVALID_AREA");
    texts.category = text("HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA");
    assert_eq!(texts.refused, "You can't construct a building here.");
    assert_eq!(texts.category, "Building doesn't fit in this area.");
    info!(
        "house_build: refused spot: {} / {}",
        texts.refused, texts.category
    );

    // the HUD's Start button: the builder is at Confirm, commit writes CSCreateHouse
    builder.build = Some(Build::confirmed(
        171,
        16_777_230,
        21_165,
        Vec3::new(VALID.0, VALID.1, 173.051_62),
        0.0,
        450_000,
    ));
    builder.commit(1.0, &mut net, &mut signals);
    assert!(builder.build.is_none());
}

fn verify_commit(mut commands: MessageReader<NetCommand>, mut run: ResMut<Run>) {
    for NetCommand(command) in commands.read() {
        let Command::House(command) = command else {
            continue;
        };
        let mut w = aa_net::wire::Writer::new();
        command.write(&mut w);
        let body = w.finish();
        if body != CAPTURE_CREATE {
            run.fail(format!(
                "CSCreateHouse differs from the capture: {} bytes",
                body.len()
            ));
            continue;
        }
        let HouseCommand::CreateHouse {
            x,
            y,
            item_id,
            money_amount,
            ..
        } = command
        else {
            run.fail("the Start button did not send CSCreateHouse".into());
            continue;
        };
        if (*x, *y, *item_id, *money_amount) != (15328.0, 14632.0, 16_777_230, 450_000) {
            run.fail(format!(
                "CSCreateHouse fields: ({x}, {y}, {item_id}, {money_amount})"
            ));
        }
        info!("house_build: CSCreateHouse 0x57 matches the capture (45 bytes)");
    }
}

fn drive(time: Res<Time>, mut houses: ResMut<Houses>, mut run: ResMut<Run>) {
    let Some(start) = run.started else { return };
    let t = time.elapsed_secs() - start;
    let steps = [
        (0.0, 1083, 4),
        (2.0, 1084, 10),
        (4.0, 1085, 15),
        (6.0, 1082, 20),
    ];
    while run.stage < steps.len() && t >= steps[run.stage].0 {
        let (_, model, actions) = steps[run.stage];
        houses.apply(&Event::Housing(HousingEvent::BuildProgress {
            tl_id: 13,
            model_id: model,
            all_actions: 20,
            current_actions: actions,
        }));
        info!("house_build: SCHouseBuildProgress model {model}, action {actions}");
        run.stage += 1;
    }
}

fn watch(
    time: Res<Time>,
    houses: Res<Houses>,
    drawn: Query<(Entity, &HouseModel)>,
    mut run: ResMut<Run>,
) {
    let snapshot = houses.by_tl_id.get(&13).map(|h| &h.snapshot);
    let mut by_model: HashMap<u32, Entity> = HashMap::new();
    for (entity, house) in &drawn {
        by_model.insert(house.model_id, entity);
    }
    match run.started {
        None => {
            if let Some(entity) = by_model.get(&STAGES[0]).copied() {
                run.started = Some(time.elapsed_secs());
                run.seen.insert(STAGES[0], entity);
                run.same_model_entity = Some(entity);
                info!("house_build: stage 0 drawn (model 1083)");
            } else if time.elapsed_secs() > 10.0 {
                run.fail("the stage-0 house never appeared".into());
                std::process::exit(1);
            }
        }
        Some(start) => {
            if by_model.is_empty() && snapshot.is_some() {
                run.fail("a model swap left a frame without a house".into());
            }
            for model in STAGES {
                if let Some(entity) = by_model.get(&model).copied() {
                    run.seen.entry(model).or_insert(entity);
                }
            }
            let t = time.elapsed_secs() - start;
            if t > 1.0 && !run.same_model_checked {
                run.same_model_checked = true;
                if by_model.get(&STAGES[0]).copied() != run.same_model_entity {
                    run.fail("progress with the same model respawned the house".into());
                }
            }
            if t >= 8.0 && !run.done {
                run.done = true;
                for model in STAGES {
                    if !run.seen.contains_key(&model) {
                        run.fail(format!("model {model} was never drawn"));
                    }
                }
                let final_model = snapshot.map(|s| s.model_id);
                if final_model != Some(STAGES[3]) {
                    run.fail(format!(
                        "the finished model is {final_model:?}, expected {}",
                        STAGES[3]
                    ));
                }
            }
        }
    }
}

fn report(mut run: ResMut<Run>) {
    if run.done && !run.reported {
        run.reported = true;
        info!(
            "house_build: checks {} - models {:?}",
            if run.failed.is_empty() {
                "passed"
            } else {
                "FAILED"
            },
            run.seen.keys().copied().collect::<Vec<_>>()
        );
        if !run.failed.is_empty() {
            std::process::exit(1);
        }
    }
}

fn banner(mut commands: Commands, mut fonts: ResMut<Assets<Font>>, texts: Res<Texts>) {
    let font = match aa_pak::Pak::global().read("ui/font/yd_ygo540.ttf") {
        Ok(bytes) => fonts.add(Font::from_bytes(bytes)),
        Err(_) => Handle::default(),
    };
    for (i, text) in [
        format!("refused {} {}: {}", REFUSED.0, REFUSED.1, texts.refused),
        format!(
            "valid {} {}: CSCreateHouse 0x57 committed",
            VALID.0, VALID.1
        ),
        "SCHouseBuildProgress: 1083 -> 1084 -> 1085 -> 1082".to_string(),
    ]
    .into_iter()
    .enumerate()
    {
        commands.spawn((
            Text::new(text),
            TextFont {
                font: bevy::text::FontSource::Handle(font.clone()),
                font_size: bevy::text::FontSize::Px(22.0),
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.92, 0.75)),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(18.0 + i as f32 * 30.0),
                left: Val::Px(18.0),
                ..default()
            },
        ));
    }
}
