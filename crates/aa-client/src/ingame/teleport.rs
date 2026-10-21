//! Teleports in game: a portal, a recall, a respawn far away, a GM move.
//!
//! The server moves the player with `SCTeleportUnit` (aa-net answers `CSTeleportEnded`, `player::follow_server` puts
//! the player there, the world streams around the new focus by itself), preceded by `SCLoadInstance` for a recall, a
//! world gate or an instance change. A short hop is just that. A long one ([`FAR`] or any `SCLoadInstance`) shows the
//! loading screen like the original client: the HUD stage is put aside (`hud::suspend`), the original loading UI is
//! built in its place (`login::loading::build`, `ENTERED_LOADING` / `STILL_LOADING`), the streaming budget is raised,
//! the player's input is off, and the gauge follows the world around the destination (terrain within [`NEAR`] metres,
//! then the object sectors, models and units). At 100 % the loading screen is captured and held over the window
//! (`login::curtain`), `LEFT_LOADING` goes out, the HUD stage comes back underneath, and the curtain is lifted once the
//! game screen is quiet. After an `SCLoadInstance` the server waits for `CSInstanceLoaded` (sent then) before it lets the
//! player move again.

use aa_net::state::Player;
use aa_net::{Command, Event, NetCommand, NetEvent, PortalCommand};
use aa_ui::UiRuntime;
use aa_world::coords::cell_origin;
use aa_world::objects::ObjectState;
use aa_world::stream::Streamer;
use aa_world::{CELL_SIZE, TerrainQuery};
use bevy::prelude::*;

use super::player::{LocalPlayer, Mover};
use super::world_switch;
use crate::Screen;
use crate::login::curtain::{self, Curtain};

/// A jump longer than this (metres) shows the loading screen.
pub const FAR: f32 = 300.0;
/// Terrain within this many metres of the destination is built before the game goes on.
const NEAR: f32 = 400.0;
/// Seconds everything must have been complete before the gauge is full.
const SETTLE: f32 = 0.4;
/// Seconds at 100 % before the curtain goes down.
const HOLD: f32 = 0.2;
/// The game goes on after this many seconds whatever is still loading.
const TIMEOUT: f32 = 20.0;

/// A teleport's loading screen.
#[derive(Resource, Default)]
pub struct Teleport {
    /// A long jump the server just made (destination), to put the loading screen up for.
    pending: Option<Vec3>,
    /// `SCLoadInstance` came: `CSInstanceLoaded` is owed once the destination is loaded.
    instance_loaded_owed: bool,
    load: Option<Load>,
}

struct Load {
    target: Vec3,
    t: f32,
    shown: f32,
    stable: f32,
    done: f32,
    peak: usize,
    /// The loading UI replaced the HUD stage (false: the HUD was not up, nothing to give back).
    hud: bool,
    left: bool,
}

impl Teleport {
    /// The loading screen of a teleport is up (the player's input is off).
    pub fn loading(&self) -> bool {
        self.load.is_some() || self.pending.is_some()
    }
}

/// Run condition: no teleport loading screen.
pub fn idle(tp: Res<Teleport>) -> bool {
    !tp.loading()
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Teleport>()
        .add_systems(
            Update,
            detect
                .before(super::player::follow_server)
                .before(aa_world::WorldSwitchSet)
                .run_if(in_state(Screen::InGame)),
        )
        .add_systems(
            Update,
            (drive, progress)
                .chain()
                .after(super::player::follow_server)
                .after(aa_world::WorldSwitchSet)
                .run_if(in_state(Screen::InGame)),
        )
        .add_systems(OnExit(Screen::InGame), |mut tp: ResMut<Teleport>, mut cfg: ResMut<aa_world::WorldConfig>| {
            if tp.load.is_some() {
                normal_streaming(&mut cfg);
            }
            *tp = Teleport::default();
        });
}

/// Before `follow_server` moves the player: is this jump a long one, and does it change the world?
fn detect(
    mut events: MessageReader<NetEvent>,
    player: Query<(&LocalPlayer, &Mover)>,
    current: Res<world_switch::CurrentWorld>,
    mut switch: ResMut<aa_world::WorldSwitch>,
    mut tp: ResMut<Teleport>,
) {
    let Ok((local, mover)) = player.single() else { return };
    for NetEvent(e) in events.read() {
        let (to, load) = match e {
            Event::LoadInstance(load) => {
                match world_switch::world_for_instance(load) {
                    Some(world) if world != current.0 => {
                        info!(
                            "teleport: SCLoadInstance {} -> world {world} (zone {})",
                            load.raw_id, load.zone_id
                        );
                        switch.request(world);
                    }
                    Some(world) => debug!("teleport: SCLoadInstance {} stays in {world}", load.raw_id),
                    None => warn!(
                        "teleport: SCLoadInstance {} names zone {}, which no game/worlds folder owns; keeping {}",
                        load.raw_id, load.zone_id, current.0
                    ),
                }
                (Vec3::new(load.pos.x, load.pos.y, load.pos.z), true)
            }
            Event::Teleported { pos, .. } => (Vec3::new(pos.x, pos.y, pos.z), false),
            Event::Resurrected { obj_id, pos, .. } if *obj_id == local.obj_id => (Vec3::new(pos.x, pos.y, pos.z), false),
            _ => continue,
        };
        if load {
            tp.instance_loaded_owed = true;
        }
        let far = mover.pos.truncate().distance(to.truncate()) > FAR;
        if load || far || tp.pending.is_some() || tp.load.is_some() {
            info!("teleport: {} jump of {:.0} m to ({:.1}, {:.1}, {:.1}){}", if far { "a long" } else { "a" }, mover.pos.truncate().distance(to.truncate()), to.x, to.y, to.z, if load { " (instance load)" } else { "" });
            tp.pending = Some(to);
        } else if !tp.instance_loaded_owed {
            debug!("teleport: a short hop of {:.1} m", mover.pos.distance(to));
        }
    }
}

const LOADING_BUDGET_MS: (f32, f32) = (10.0, 4.0);

fn boost_streaming(cfg: &mut aa_world::WorldConfig) {
    (cfg.object_budget_ms, cfg.finish_budget_ms) = LOADING_BUDGET_MS;
}

fn normal_streaming(cfg: &mut aa_world::WorldConfig) {
    let defaults = aa_world::WorldConfig::new(&cfg.world);
    (cfg.object_budget_ms, cfg.finish_budget_ms) = (defaults.object_budget_ms, defaults.finish_budget_ms);
}

/// Puts the loading screen up for a pending jump, and takes it down when [`progress`] is done.
fn drive(world: &mut World) {
    let pending = world.resource_mut::<Teleport>().pending.take();
    if let Some(target) = pending {
        let mut tp = world.resource_mut::<Teleport>();
        if let Some(load) = tp.load.as_mut() {
            // another jump while loading: wait for that place instead
            *load = Load { target, hud: load.hud, ..new_load(target) };
        } else {
            let hud = crate::hud::suspend(world);
            if hud && world.get_non_send::<crate::login::LoginBridge>().is_some() {
                crate::login::loading::build(world);
            } else if hud {
                // `--enter-world` has no login stage to build the loading UI with: an empty stage while loading (not
                // `host::clear`, which turns the UI cameras off and the HUD would come back invisible)
                crate::login::host::switch_stage(world, |_| {});
            }
            boost_streaming(&mut world.resource_mut::<aa_world::WorldConfig>());
            world.resource::<TerrainQuery>().prefetch(target.x, target.y, NEAR + 100.0);
            world.resource_mut::<Teleport>().load = Some(Load { hud, ..new_load(target) });
            info!("teleport: loading screen up (HUD put aside: {hud})");
        }
    }
    let left = world.resource::<Teleport>().load.as_ref().is_some_and(|l| l.left);
    if !left {
        return;
    }
    let load = world.resource_mut::<Teleport>().load.take().expect("load");
    if load.hud && !crate::hud::resume(world) {
        warn!("teleport: no HUD stage to give back");
    }
    normal_streaming(&mut world.resource_mut::<aa_world::WorldConfig>());
    if std::mem::take(&mut world.resource_mut::<Teleport>().instance_loaded_owed) {
        world.write_message(NetCommand(Command::Portal(PortalCommand::InstanceLoaded)));
    }
    info!("teleport: arrived after {:.1} s at ({:.1}, {:.1}, {:.1})", load.t, load.target.x, load.target.y, load.target.z);
}

fn new_load(target: Vec3) -> Load {
    Load { target, t: 0.0, shown: 0.0, stable: 0.0, done: 0.0, peak: 0, hud: false, left: false }
}

/// Cells within [`NEAR`] metres of a game position that exist.
fn near_cells(terrain: &TerrainQuery, x: f32, y: f32) -> Vec<(i32, i32)> {
    let (c0x, c0y) = (((x - NEAR) / CELL_SIZE).floor() as i32, ((y - NEAR) / CELL_SIZE).floor() as i32);
    let (c1x, c1y) = (((x + NEAR) / CELL_SIZE).floor() as i32, ((y + NEAR) / CELL_SIZE).floor() as i32);
    let mut cells = Vec::new();
    for cy in c0y..=c1y {
        for cx in c0x..=c1x {
            let o = cell_origin(cx, cy);
            let dx = (o.x - x).max(0.0).max(x - (o.x + CELL_SIZE));
            let dy = (o.y - y).max(0.0).max(y - (o.y + CELL_SIZE));
            if (dx * dx + dy * dy).sqrt() <= NEAR && terrain.cell(cx, cy).is_some() {
                cells.push((cx, cy));
            }
        }
    }
    cells
}

/// The gauge: the terrain around the destination (0..60), then what is still being built there (60..95), then
/// [`SETTLE`] seconds of nothing new (95..100). At 100 the curtain goes down; once it is drawn, `LEFT_LOADING`.
#[allow(clippy::too_many_arguments)]
fn progress(
    time: Res<Time>,
    rt: Option<NonSend<UiRuntime>>,
    terrain: Res<TerrainQuery>,
    streamer: Option<Res<Streamer>>,
    objects: Option<Res<ObjectState>>,
    warm: Res<super::warm::Warm>,
    player: Res<Player>,
    mut tp: ResMut<Teleport>,
    curtain: Option<ResMut<Curtain>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let Some(load) = tp.load.as_mut() else { return };
    if load.left {
        return;
    }
    load.t += dt;
    let needed = near_cells(&terrain, load.target.x, load.target.y);
    let loaded = streamer.as_ref().map_or(0, |s| {
        let have: Vec<(i32, i32)> = s.loaded_cells().collect();
        needed.iter().filter(|c| have.contains(c)).count()
    });
    let stats = objects.as_ref().map(|o| o.stats.clone()).unwrap_or_default();
    let busy = stats.sectors_incomplete + stats.instances_waiting + stats.models_pending + stats.cells_pending + warm.outstanding;
    let terrain_done = loaded >= needed.len();
    let far_terrain = streamer.as_ref().is_some_and(|s| s.is_loading());
    load.peak = load.peak.max(busy);
    let all_done = terrain_done && busy == 0 && !far_terrain;
    load.stable = if all_done { load.stable + dt } else { 0.0 };
    let target = if load.t > TIMEOUT || load.stable >= SETTLE || player.spawn.is_none() {
        if load.t > TIMEOUT && load.shown < 100.0 {
            warn!("teleport: the destination is not ready after {TIMEOUT} s ({loaded}/{} cells, {busy} outstanding): going on", needed.len());
        }
        100.0
    } else if !terrain_done {
        60.0 * loaded as f32 / needed.len().max(1) as f32
    } else if all_done {
        95.0 + 5.0 * (load.stable / SETTLE).min(0.9)
    } else {
        60.0 + 35.0 * (1.0 - busy as f32 / load.peak.max(1) as f32).min(0.99)
    };
    if target >= load.shown + 1.0 || (target >= 100.0 && load.shown < 100.0) {
        load.shown = target;
        if let Some(rt) = rt.as_deref().filter(|_| load.hud) {
            rt.fire_event("STILL_LOADING", f64::from(target));
        }
    }
    if load.shown < 100.0 {
        return;
    }
    load.done += dt;
    if load.done < HOLD {
        return;
    }
    // no curtain without the login stage (`--enter-world`)
    let (true, Some(mut curtain)) = (load.hud, curtain) else {
        if let Some(rt) = rt.as_deref().filter(|_| load.hud) {
            rt.fire_event("LEFT_LOADING", ());
        }
        load.left = true;
        return;
    };
    // the loading screen at 100 stays on the window while the HUD stage comes back underneath
    if curtain.phase == curtain::Phase::Off {
        curtain::lower(&mut commands, &mut curtain);
        return;
    }
    if curtain.phase == curtain::Phase::Capturing {
        curtain::tick(&mut curtain, dt);
    }
    if !curtain.ready() {
        return;
    }
    if let Some(rt) = rt.as_deref() {
        rt.fire_event("LEFT_LOADING", ());
    }
    info!("teleport: the destination is loaded ({loaded}/{} cells, {} object entities) after {:.1} s", needed.len(), stats.entities, load.t);
    load.left = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::state::Player;
    use aa_net::InstanceLoad;

    fn player_spawn() -> aa_net::PlayerSpawn {
        aa_net::PlayerSpawn {
            obj_id: 100,
            state: aa_net::model::CharacterState {
                character: aa_net::Character { id: 47, level: 30, ..default() },
                ..default()
            },
            unit: aa_net::Unit {
                bounty_money: None,
                pvp_honor: 0,
                pvp_kills: 0,
                obj_id: 100,
                name: "Teleport fixture".into(),
                kind: aa_net::model::UnitKind::Character { char_id: 47 },
                owner_name: String::new(),
                pos: aa_net::Pos::new(10.0, 20.0, 30.0),
                rotation: [0.0; 3],
                scale: 1.0,
                level: 30,
                model_id: 0,
                equipment: default(),
                model_params: default(),
                hp: 100,
                mp: 100,
                max_hp: 100,
                max_mp: 100,
                dead: false,
                race_gender: 1,
                movement: None,
                faction_id: 0,
                expedition_id: 0,
                appellation: 0,
                flags: 0,
                gm_modes: [0; 4],
                skills: vec![],
                passive_buffs: vec![],
                ability_exp: vec![],
                active_abilities: vec![],
                buffs: vec![],
                anim_action: 0,
                attached_to: None,
                house_posture: None,
                state_attachments: None,
            },
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(0.25)))
            .insert_state(Screen::InGame)
            .add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .insert_resource(Player { spawn: Some(player_spawn()) })
            .insert_resource(aa_world::WorldConfig::new("main_world"))
            .insert_resource(aa_world::stream::WorldInfo(std::sync::Arc::new(aa_world::World::parse_g(
                "name main_world\ncellX 1\ncellY 1\nzones\n zone ( name w_solzreed_1, id 142 )\n",
            ))))
            .insert_resource(aa_world::TerrainQuery::new("main_world"))
            .init_resource::<aa_world::WorldSwitch>()
            .init_resource::<world_switch::CurrentWorld>()
            .init_resource::<super::super::warm::Warm>()
            .init_resource::<crate::hud::HudBuilt>()
            .add_plugins(plugin);
        app.world_mut().spawn((
            LocalPlayer { obj_id: 100 },
            Mover {
                pos: Vec3::new(10.0, 20.0, 30.0),
                facing: 0.0,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
            Transform::default(),
        ));
        app
    }

    fn instance_load() -> InstanceLoad {
        InstanceLoad {
            raw_id: 0,
            zone_id: 0,
            pos: aa_net::Pos::new(10.0, 20.0, 30.0),
            angles: [0.0; 3],
        }
    }

    #[test]
    fn an_instance_load_requests_its_world_folder() {
        if !aa_world::pak_available() {
            return;
        }
        let mut app = app();
        let mut load = instance_load();
        load.raw_id = 7;
        load.zone_id = 198;
        app.world_mut().write_message(NetEvent(Event::LoadInstance(load)));
        app.update();
        assert_eq!(
            app.world().resource::<aa_world::WorldSwitch>().pending(),
            Some("instance_training_camp")
        );
    }

    #[test]
    fn instance_loaded_is_sent_after_the_destination_is_ready() {
        let mut app = app();
        app.world_mut().write_message(NetEvent(Event::LoadInstance(instance_load())));
        for _ in 0..12 {
            app.update();
            if app.world().resource::<Teleport>().pending.is_none()
                && app.world().resource::<Teleport>().load.is_none()
            {
                break;
            }
        }
        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|v| v.0)
            .collect();
        assert!(
            sent.iter().any(|c| matches!(c, Command::Portal(PortalCommand::InstanceLoaded))),
            "the server is answered after the loading screen: {sent:?}"
        );
        assert!(!app.world().resource::<Teleport>().loading());
        assert_eq!(app.world().resource::<world_switch::CurrentWorld>().0, "main_world");
    }
}
