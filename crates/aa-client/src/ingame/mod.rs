//! In game: world around the player, the player character, other units, camera and movement.
//!
//! * [`enter`]: `--enter-world`, logs in and enters the game without the login screens
//! * [`player`]: the local player (spawn from the server's data, keyboard movement, collision, movement packets)
//! * [`swim`]: swimming (when it starts, the swim clips, the actor parameters of the database)
//! * [`camera`]: ArcheAge's third person orbit camera
//! * [`units`]: other players, NPCs and doodads as the server reports them
//! * [`doodad_visual`]: the model and growth scale of a doodad's current phase
//! * [`anim`], [`creature`], [`appearance`]: locomotion clips, non-player models, looks from the server
//! * [`combat`]: attack and cast clips, skill effects, projectiles, hit reactions and death, from what the server reports
//! * [`skill_motion`]: skills that move units (charges, leaps, pulls, knockbacks, blinks) and the camera that follows them
//! * [`data`]: game database lookups (ground heights come from aa-world's `TerrainQuery`)
//! * [`flow`], [`warm`]: the way in and out timed (`flow:` log lines), and what the world behind the loading screen has
//!   got done
//! * [`prewarm`]: the world's render pipelines compiled while the player sits on the character list
//!
//! ## Entering and leaving
//!
//! The world is built on the loading screen: as soon as the server's spawn is known `player::spawn_player` builds the
//! character and `camera::spawn_camera` the world camera, which renders into an image of the window's size
//! (`camera::BehindLoading`); the units near the spawn are prepared by `units`, and `login::loading` waits until the
//! terrain, the objects, the units and the GPU pipelines (`aa_world::pipelines::PipelineStatus`) have settled. The first-use
//! work of the renderer thus happens behind the loading UI. `Screen::InGame` then turns the input on and
//! `camera::show_world` puts the camera on the window once the UI cameras agree with it (HDR, MSAA), all of it under the
//! loading screen's curtain (`login::curtain`: a capture of the loading screen at 100 % held over the window until the HUD,
//! the name plates and the quest marks are in). [`trace`] (`AA_ENTER_TRACE=1`) measures what still comes up after.
//! `--enter-world` has no loading screen: the camera goes straight to the window.
//!
//! Leaving (`OnExit(Screen::InGame)`, [`leave_game`]) removes the player, the units, the doodads and the camera, forgets
//! the unit views, puts the UI cameras back to what the login screens need and gives the world its anchor back, so the
//! cells unload. The character list is then built again by `login`.

mod anim;
pub(crate) mod attachments;
mod bench;
pub(crate) mod appearance;
pub(crate) mod camera;
pub(crate) mod combat;
pub(crate) mod creature;
pub(crate) mod data;
mod dev;
mod doodad_visual;
pub(crate) mod emote;
pub(crate) mod enter;
mod flow;
pub(crate) mod ground_target;
mod idle;
mod mount;
pub(crate) mod player;
mod prepare;
mod prewarm;
mod remote;
mod skill_motion;
mod sounds;
mod stealth;
mod swim;
pub(crate) mod teleport;
mod trace;
pub(crate) mod units;
pub(crate) mod warm;
pub(crate) mod weapons;

use aa_anim::AnimPlugin;
use aa_net::NetPlugin;
use aa_world::AaWorldPlugin;
use bevy::prelude::*;

use crate::Screen;
pub(crate) use dev::DevOptions;

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
        if !app.is_plugin_added::<aa_anim::facial::FacialPlugin>() {
            app.add_plugins(aa_anim::facial::FacialPlugin);
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
            .init_resource::<player::PlayerBuilt>()
            .init_resource::<warm::Warm>()
            .add_plugins(enter::plugin)
            .add_plugins(emote::plugin)
            .add_plugins(combat::plugin)
            .add_plugins(ground_target::plugin)
            .add_plugins(skill_motion::plugin)
            .add_plugins(sounds::plugin)
            .add_plugins(stealth::plugin)
            .add_plugins(flow::plugin)
            .add_plugins(prewarm::plugin)
            .add_plugins(units::plugin)
            .add_plugins(doodad_visual::plugin)
            .add_plugins(attachments::plugin)
            .add_plugins(bench::plugin)
            .add_plugins(trace::plugin)
            .add_plugins(teleport::plugin)
            .add_systems(Startup, fix_window)
            .add_systems(Last, report_anim.run_if(resource_exists::<aa_world::profile::Timers>))
            .add_systems(OnExit(Screen::InGame), leave_game)
            // the world is built on the loading screen (drawn into an image behind it) and played on the game screen
            .add_systems(
                Update,
                (
                    (player::spawn_player, camera::spawn_camera).chain(),
                    (
                        camera::orbit_input.run_if(in_state(Screen::InGame)).run_if(teleport::idle),
                        player::follow_server,
                        player::control_player.run_if(in_state(Screen::InGame)).run_if(teleport::idle),
                        camera::follow_player,
                    )
                        .chain()
                        .after(crate::hud::HudInputReady),
                    (units::sync_views, units::move_units, units::cull_units).chain(),
                    anim::drive_locomotion,
                    swim::drive_swim.after(anim::drive_locomotion),
                    player::dev_hide_part,
                    weapons::attach_carried,
                    warm::update,
                    sync_clock,
                    log_chat,
                    report,
                )
                    .run_if(world_screens),
            )
            .add_systems(Update, (camera::show_world, leave_after).run_if(in_state(Screen::InGame)));
    }
}

/// The world's time of day follows the server's game clock (`aa_net::state::WorldInfo`, extrapolated from its last packet: the
/// HUD's clock reads the same), on the loading screen as well as in game. Nothing happens until the server has sent the time;
/// `--hour H` pins the hour for screenshot tests.
fn sync_clock(info: Option<Res<aa_net::state::WorldInfo>>, dev: Res<dev::DevOptions>, mut clock: ResMut<aa_world::WorldClock>, mut logged: Local<bool>) {
    if dev.hour.is_some() {
        return;
    }
    let Some(hour) = info.and_then(|w| w.hour_now()) else { return };
    if (clock.hour - hour).abs() > 1e-5 {
        clock.hour = hour;
    }
    if !*logged {
        *logged = true;
        info!("world clock follows the server: {hour:.2} h");
    }
}

/// The character list is up and built (`--enter-world` has no login stage: no `UiBuilt`, no list).
pub(crate) fn list_is_up(built: Option<Res<crate::login::UiBuilt>>, screen: Res<State<Screen>>) -> bool {
    *screen.get() == Screen::CharacterSelect && built.is_some_and(|b| b.0 == Some(Screen::CharacterSelect))
}

/// The screens the world is up on: the loading screen (everything is built behind it) and the game.
pub(crate) fn world_screens(screen: Res<State<Screen>>) -> bool {
    matches!(screen.get(), Screen::Loading | Screen::InGame)
}

/// `OnExit(Screen::InGame)` (back to the character or server list): removes everything of the game screen, the world follows
/// by itself once its focus is far away. The player, the other units and doodads, the orbit camera (the world's focus and post
/// processing target) go; the views of the units are forgotten, the movement packet state starts over, and the world gets its
/// anchor back (see `login::WorldAnchor`: an inactive camera far outside the map, so nothing streams and the loaded cells unload).
fn leave_game(
    mut commands: Commands,
    doomed: Query<Entity, Or<(With<player::LocalPlayer>, With<units::RemoteUnit>, With<units::RemoteDoodad>, With<camera::OrbitCamera>)>>,
    anchor: Query<(), With<crate::login::WorldAnchor>>,
    ui_cameras: Query<Entity, With<aa_ui::render::UiCamera>>,
) {
    let mut n = 0;
    for e in &doomed {
        commands.entity(e).despawn();
        n += 1;
    }
    commands.insert_resource(units::Views::default());
    commands.insert_resource(player::SendState::default());
    commands.insert_resource(player::PlayerBuilt::default());
    commands.insert_resource(warm::Warm::default());
    // the HUD made the UI cameras agree with the world camera (HDR, no MSAA, `hud::cameras`): the login stage camera
    // below them does not, and a stack of cameras that disagree on the sample count is a validation error
    for e in &ui_cameras {
        commands.entity(e).remove::<bevy::camera::Hdr>().insert(bevy::render::view::Msaa::default());
    }
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
    let (far, failed, pending, _) = views.census(&net_units, p.pos);
    info!(
        "in game: player ({:.1}, {:.1}, {:.1}) | server units {} (drawn {}, built {}, failed {}; not drawn: {far} beyond {} m, {failed} without a model, {pending} waiting) doodads {} (drawn {}, built {})",
        p.pos.x,
        p.pos.y,
        p.pos.z,
        net_units.by_id.len(),
        views.unit_entities(),
        views.spawned_units,
        views.failed,
        units::UNIT_RADIUS,
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
