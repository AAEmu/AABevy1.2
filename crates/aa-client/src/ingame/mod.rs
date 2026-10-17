//! In game: world around the player, the player character, other units, camera and movement.
//!
//! * [`enter`]: `--enter-world`, logs in and enters the game without the login screens
//! * [`player`]: the local player (spawn from the server's data, keyboard movement, movement packets)
//! * [`camera`]: ArcheAge's third person orbit camera
//! * [`units`]: other players, NPCs and doodads as the server reports them
//! * [`anim`], [`creature`], [`appearance`]: locomotion clips, non-player models, looks from the server
//! * [`data`]: game database lookups (ground heights come from aa-world's `TerrainQuery`)

mod anim;
mod bench;
pub(crate) mod appearance;
mod camera;
mod creature;
pub(crate) mod data;
mod dev;
pub(crate) mod enter;
mod player;
mod prepare;
mod units;
mod weapons;

use aa_anim::AnimPlugin;
use aa_net::NetPlugin;
use aa_world::AaWorldPlugin;
use bevy::prelude::*;

use crate::Screen;

/// The world the in-game screen plays in.
pub const WORLD: &str = "main_world";

pub struct InGamePlugin;

impl Plugin for InGamePlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<NetPlugin>() {
            app.add_plugins(NetPlugin);
        }
        if !app.is_plugin_added::<aa_cry::CryPlugin>() {
            app.add_plugins(aa_cry::CryPlugin);
        }
        if !app.is_plugin_added::<AnimPlugin>() {
            app.add_plugins(AnimPlugin);
        }
        if !app.is_plugin_added::<AaWorldPlugin>() {
            app.add_plugins(AaWorldPlugin::new(WORLD));
        }
        if let Some(db) = data::Db::open() {
            app.insert_resource(db);
        }
        if let Some(auto) = enter::AutoEnter::from_args() {
            app.insert_resource(auto);
        }
        let dev = dev::DevOptions::from_args();
        if let Some(mut clock) = app.world_mut().get_resource_mut::<aa_world::WorldClock>() {
            if let Some(h) = dev.hour {
                clock.hour = h;
            }
            if dev.zone.is_some() {
                clock.zone = dev.zone;
            }
        }
        if aa_world::profile::requested() && !app.is_plugin_added::<aa_world::profile::ProfilePlugin>() {
            app.add_plugins(aa_world::profile::ProfilePlugin);
        }
        app.insert_resource(dev)
            .init_resource::<units::Views>()
            .init_resource::<player::SendState>()
            .add_plugins(enter::plugin)
            .add_plugins(bench::plugin)
            .add_systems(Startup, fix_window)
            .add_systems(Last, report_anim.run_if(resource_exists::<aa_world::profile::Timers>))
            .add_systems(OnEnter(Screen::InGame), player::spawn_player)
            .add_systems(OnExit(Screen::InGame), leave_game)
            .add_systems(
                Update,
                (
                    camera::spawn_camera,
                    (camera::orbit_input, player::follow_server, player::control_player, camera::follow_player).chain(),
                    (units::sync_views, units::move_units, units::cull_units).chain(),
                    anim::drive_locomotion,
                    player::dev_hide_part,
                    weapons::attach_carried,
                    log_chat,
                    report,
                    leave_after,
                )
                    .run_if(in_state(Screen::InGame)),
            );
    }
}

/// `OnExit(Screen::InGame)` (back to the character or server list): removes everything of the game screen, the world follows
/// by itself once its focus is far away. The player, the other units and doodads, the orbit camera (the world's focus and post
/// processing target) go; the views of the units are forgotten, the movement packet state starts over, and the world gets its
/// anchor back (see `login::WorldAnchor`: an inactive camera far outside the map, so nothing streams and the loaded cells unload).
fn leave_game(
    mut commands: Commands,
    doomed: Query<Entity, Or<(With<player::LocalPlayer>, With<units::RemoteUnit>, With<units::RemoteDoodad>, With<camera::OrbitCamera>)>>,
    anchor: Query<(), With<crate::login::WorldAnchor>>,
) {
    let mut n = 0;
    for e in &doomed {
        commands.entity(e).despawn();
        n += 1;
    }
    commands.insert_resource(units::Views::default());
    commands.insert_resource(player::SendState::default());
    if anchor.is_empty() {
        commands.spawn((
            Name::new("world anchor"),
            crate::login::WorldAnchor,
            aa_world::WorldFocus,
            Camera3d::default(),
            Camera { is_active: false, ..default() },
            Transform::from_xyz(-1.0e6, 0.0, 1.0e6),
        ));
    }
    info!("left the game screen: {n} entities removed");
}

/// `--leave-after S`: go back to the character list S seconds after the player appeared (client side only; checks
/// [`leave_game`]).
fn leave_after(time: Res<Time>, dev: Res<dev::DevOptions>, mut since: Local<f32>, mut next: ResMut<NextState<Screen>>) {
    let Some(after) = dev.leave_after else { return };
    *since += time.delta_secs();
    if *since >= after {
        next.set(Screen::CharacterSelect);
    }
}

/// Feeds aa-anim's counters into the profile timers (`AA_PROFILE=1`).
fn report_anim(timers: Res<aa_world::profile::Timers>) {
    let (updated, nanos) = aa_anim::animator::take_stats();
    timers.add("anim.animate", nanos as f64 / 1e6);
    timers.add("anim.animators(count)", f64::from(updated));
}

/// A line every few seconds: where the player is and how much of the town is drawn.
fn report(
    time: Res<Time>,
    mut since: Local<f32>,
    views: Res<units::Views>,
    player: Query<&player::Mover, With<player::LocalPlayer>>,
    net_units: Res<aa_net::state::Units>,
    net_doodads: Res<aa_net::state::Doodads>,
) {
    *since += time.delta_secs();
    if *since < 5.0 {
        return;
    }
    *since = 0.0;
    let Ok(p) = player.single() else { return };
    info!(
        "in game: player ({:.1}, {:.1}, {:.1}) | server units {} (drawn {}, built {}, failed {}) doodads {} (drawn {}, built {})",
        p.pos.x,
        p.pos.y,
        p.pos.z,
        net_units.by_id.len(),
        views.unit_entities(),
        views.spawned_units,
        views.failed,
        net_doodads.by_id.len(),
        views.doodad_entities(),
        views.spawned_doodads
    );
}

/// Unattended runs (`--enter-world` / `--shot`) want one window pixel per pixel, whatever the
/// desktop scaling: the real client is compared pixel for pixel.
fn fix_window(mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>, args: Option<Res<aa_dev::DevArgs>>, auto: Option<Res<enter::AutoEnter>>) {
    let Some(args) = args else { return };
    if args.shot.is_none() && auto.is_none() {
        return;
    }
    for mut w in &mut windows {
        w.resolution.set_scale_factor_override(Some(1.0));
        w.resolution.set(args.size.0 as f32, args.size.1 as f32);
    }
}

/// What the server says: the welcome message arrives with the first movement.
fn log_chat(mut events: MessageReader<aa_net::NetEvent>) {
    for aa_net::NetEvent(event) in events.read() {
        match event {
            aa_net::Event::Chat(c) => info!("chat [{}] {}: {}", c.chat_type, c.sender, c.message),
            aa_net::Event::ServerError { code } => warn!("server error {code}"),
            _ => {}
        }
    }
}
