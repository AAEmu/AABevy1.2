//! Original UI plus all six actual service systems; no server. The CPU harness
//! is also used by the optional aa-dev visual example.
use super::*;
use crate::hud::{
    self,
    api::Shared,
    data::{GameData, SharedGameData},
};
use aa_net::state::{Buffs, PlayerStats, Quests, Skills};

pub(super) const A: u32 = 0x010203;
pub(super) const B: u32 = 0x010204;
pub(super) const TEMPLATE: u32 = 1024;

pub(super) fn harness() -> Result<(App, Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
    harness_at(1280., 720.)
}

fn harness_at(
    width: f32,
    height: f32,
) -> Result<(App, Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
    let (rt, shared, path, inventory) = hud::expansion::fixture::runtime(width, height)?;
    // Synthetic cross-service metadata makes every former contains-opener eligible.
    // It is deliberately not a retail offer or NPC policy fixture.
    let mut game = GameData::load(aa_anim::db::DEFAULT_DB)?;
    assert!(game.expedition.is_creator(TEMPLATE));
    game.quests = default();
    game.pets.stables.insert(TEMPLATE);
    game.stores.bankers.insert(TEMPLATE);
    game.stores.auctioneers.insert(TEMPLATE);
    let good = 5569;
    assert!(game.items.contains_key(&good));
    game.stores.merchants.insert(
        TEMPLATE,
        vec![hud::store::Good {
            item: good,
            grade: 0,
            currency: 0,
        }],
    );
    let db = rusqlite::Connection::open_in_memory()?;
    db.execute_batch("CREATE TABLE npcs(id,recruiting_battle_field_id);INSERT INTO npcs VALUES(1024,1);CREATE TABLE localized_texts(idx,tbl_column_name,en_us,tbl_name);CREATE TABLE battle_fields(id,name,desc);INSERT INTO battle_fields VALUES(1,'Fixture','Fixture');CREATE TABLE game_rule_sets(battle_field_id,level_min,corps_size,corps1_id,corps2_id,victory_score,victory_kill_count,victory_kill_corps1_head,id);INSERT INTO game_rule_sets VALUES(1,1,10,1,2,100,0,0,1);CREATE TABLE tagged_buffs(buff_id,tag_id);CREATE TABLE tagged_skills(skill_id,tag_id);")?;
    game.battlefield = hud::battlefield::Tables::load(&db)?;
    {
        let mut data = shared.borrow_mut();
        data.game = SharedGameData::default();
        assert!(data.game.0.set(game).is_ok());
        data.pos = Vec3::ZERO;
        data.panels.target = Some(hud::panels::TargetSnap {
            obj_id: A,
            npc_type: Some(TEMPLATE),
            detail: "npc",
            ..default()
        });
        data.target_pos = Some(Vec3::X);
    }
    rt.lua.load("NPC_KINDS={};NPC_ENDS=0;SALON_CALLS=0;UIParent:SetEventHandler('NPC_INTERACTION_START',function(kind) table.insert(NPC_KINDS,kind) end);UIParent:SetEventHandler('NPC_INTERACTION_END',function() NPC_ENDS=NPC_ENDS+1 end);local confirm=SALON_CONFIRM;SALON_CONFIRM=function(...) SALON_CALLS=SALON_CALLS+1;return confirm(...) end").exec()?;
    let mut spawn = hud::expansion::fixture::spawn();
    spawn.state.character.faction_id = 101;
    spawn.unit.faction_id = 101;
    let mut units = Units::default();
    for id in [A, B] {
        let mut npc = spawn.unit.clone();
        npc.obj_id = id;
        npc.kind = UnitKind::Npc {
            template_id: TEMPLATE,
            owner_id: 0,
        };
        npc.pos = aa_net::Pos::new(1., 0., 0.);
        npc.dead = false;
        units.by_id.insert(id, npc);
    }
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
        .insert_state(crate::Screen::InGame)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(shared.clone()))
        .insert_resource(hud::HudBuilt {
            up: true,
            fresh: false,
        })
        .init_resource::<hud::HudSuspended>()
        .insert_resource(Player {
            spawn: Some(spawn.clone()),
        })
        .insert_resource(units)
        .insert_resource(inventory)
        .init_resource::<PlayerStats>()
        .init_resource::<Quests>()
        .init_resource::<Skills>()
        .init_resource::<Buffs>()
        .init_resource::<aa_net::shop::Shop>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
        .init_resource::<aa_cry::scene::CryCache>()
        .insert_resource(crate::ingame::player::PlayerBuilt(false))
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, track_npc.before(super::super::handle_dialog))
        .add_systems(
            Update,
            (super::super::handle_dialog, drive_npc)
                .chain()
                .before(aa_ui::UiFrame),
        );
    hud::store::plugin(&mut app);
    hud::auction::plugin(&mut app);
    hud::battlefield::plugin(&mut app);
    hud::expedition::plugin(&mut app);
    hud::pets::plugin(&mut app);
    hud::salon::plugin(&mut app);
    hud::battlefield::refresh(app.world(), &mut shared.borrow_mut(), &spawn);
    for event in [
        Event::CharacterSelected(Box::new(spawn.state.clone())),
        Event::PlayerSpawn(Box::new(spawn)),
        Event::EnteredWorld,
    ] {
        app.world_mut().write_message(NetEvent(event));
    }
    app.world_mut().write_message(NetEvent(Event::Expedition(
        aa_net::ExpeditionEvent::Factions(vec![
            aa_net::ExpeditionFaction {
                id: 101,
                mother_id: 148,
                name: b"Fixture faction".to_vec(),
                ..default()
            },
            aa_net::ExpeditionFaction {
                id: 148,
                name: b"Fixture alliance".to_vec(),
                ..default()
            },
        ]),
    )));
    for unit in app
        .world()
        .resource::<Units>()
        .iter()
        .cloned()
        .collect::<Vec<_>>()
    {
        app.world_mut()
            .write_message(NetEvent(Event::UnitSpawned(Box::new(unit))));
    }
    app.update();
    drain(&mut app);
    Ok((app, shared, path))
}

pub(super) fn drain(app: &mut App) -> Vec<Command> {
    app.world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|c| c.0)
        .collect()
}

pub(super) fn interactions(commands: &[Command]) -> Vec<Interact> {
    commands
        .iter()
        .filter_map(|c| {
            if let Command::Interact(i) = c {
                Some(i.clone())
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn reply(id: u32, skills: &[u32], interactable: bool) -> Event {
    Event::Interaction(InteractEvent::SkillList {
        npc_obj_id: id,
        obj_id: 0,
        extra_info: 0,
        pick_id: 0,
        mouse_button: 2,
        modifier_keys: 0,
        interactable,
        skills: skills.to_vec(),
    })
}

pub(super) fn query(app: &mut App, shared: &Shared, id: u32) {
    query_npc(&mut shared.borrow_mut(), (id, TEMPLATE), None);
    app.update();
    let commands = drain(app);
    assert_eq!(
        interactions(&commands),
        [Interact::Start {
            npc_obj_id: id,
            obj_id: 0,
            extra_info: 0,
            pick_id: 0,
            mouse_button: 2,
            modifier_keys: 0
        }]
    );
}

/// Standard renderer controls, driven first through the actual CPU service systems.
pub(crate) fn run() {
    run_inner().unwrap();
}

fn run_inner() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (mut app, shared, path) = harness_at(width as f32, height as f32)?;
    let scene = if has("--closed") {
        "closed"
    } else if has("--auction") {
        "auction"
    } else if has("--empty") {
        "empty"
    } else if has("--ended") {
        "ended"
    } else {
        "store"
    };
    if scene != "closed" {
        query(&mut app, &shared, A);
        app.world_mut()
            .write_message(NetEvent(reply(A, &[12087, 12083], true)));
        app.update();
        assert_eq!(
            interactions(&drain(&mut app)),
            [Interact::Npc {
                obj_id: A,
                target_changed: true
            }]
        );
        if scene == "auction" {
            app.world_mut()
                .resource_mut::<Time<Virtual>>()
                .advance_by(std::time::Duration::from_secs(1));
            app.world()
                .non_send::<UiRuntime>()
                .lua
                .load("X2Interaction:SelectInteraction(12083)")
                .exec()?;
            app.update();
            app.update();
            assert_eq!(
                interactions(&drain(&mut app)),
                [Interact::Npc {
                    obj_id: A,
                    target_changed: false
                }]
            );
            app.world()
                .non_send::<UiRuntime>()
                .lua
                .load("assert(IsActivatedAuction() and not IsActivatedStore())")
                .exec()?;
        } else if scene == "empty" {
            query(&mut app, &shared, B);
            app.world_mut().write_message(NetEvent(reply(B, &[], true)));
            app.update();
            assert!(interactions(&drain(&mut app)).is_empty());
            app.world()
                .non_send::<UiRuntime>()
                .lua
                .load("assert(IsActivatedStore())")
                .exec()?;
        } else if scene == "ended" {
            app.world()
                .non_send::<UiRuntime>()
                .lua
                .load("X2Interaction:CancelNPCInteraction()")
                .exec()?;
            app.update();
            assert_eq!(
                interactions(&drain(&mut app)),
                [Interact::NpcEnd { obj_id: A }]
            );
            app.world()
                .non_send::<UiRuntime>()
                .lua
                .load("assert(not IsActivatedStore() and not IsActivatedAuction())")
                .exec()?;
        }
    }
    let rt = app.world_mut().remove_non_send::<UiRuntime>().unwrap();
    assert!(
        rt.errors.is_empty(),
        "initial script errors: {:?}",
        rt.errors
    );
    drop(app);
    println!(
        "NPC fixture: scene={scene}; actual six CPU service systems + original UI; synthetic received offers/NPC metadata, no network or retail admission proof. Secondary choice is programmatic; native selector remains separately audited."
    );
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original NPC UI - synthetic received offers"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(shared))
        .add_systems(Startup, physical_pixels)
        .run();
    std::fs::remove_file(path)?;
    Ok(())
}

fn physical_pixels(mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
