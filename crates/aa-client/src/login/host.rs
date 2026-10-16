//! The UI runtime the login screens share: one Lua state per stage, like the real client (which
//! rebuilds the whole UI state on every stage change).
//!
//! aa-ui keeps one [`UiRuntime`] as a non-send resource. [`switch_stage`] replaces it with a fresh
//! one, lets the screen install its game API and load its scripts, and makes the renderer redraw.
//! The screens call it from exclusive systems (see `build` in each screen module).
//!
//! The character create screen (owned by `login::create`) uses the same call:
//!
//! ```ignore
//! host::switch_stage(world, |rt| {
//!     // register X2LoginCharacter, X2Customizer, ... then
//!     rt.run_stage(aa_ui::Stage::CharacterCreate);
//!     rt.fire_event("SHOW_CHARACTER_CREATE_WINDOW", true);
//! });
//! ```

use aa_ui::render::{UiCamera, UiRenderState};
use aa_ui::{UiConfig, UiRuntime};
use bevy::camera::ClearColorConfig;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::stage3d::BACKGROUND_CAMERA_ORDER;

/// Where the game database with the UI texts is.
const DB: &str = aa_anim::db::DEFAULT_DB;

/// The toolkit settings of the login screens: the painted `background` layer is drawn by its own
/// camera below the 3D stage camera ([`super::stage3d::STAGE_CAMERA_ORDER`]), everything else above it.
pub fn ui_config() -> UiConfig {
    UiConfig {
        locale: "en_us".into(),
        camera_order: 10,
        clear: ClearColorConfig::Custom(Color::BLACK),
        gamma_blending: true,
        background_camera_order: Some(BACKGROUND_CAMERA_ORDER),
        background_render_layer: 31,
        ui_texts_db: Some(std::env::var("AA_DB").unwrap_or_else(|_| DB.to_string()).into()),
    }
}

fn window_size(world: &mut World) -> Option<(f32, f32)> {
    let mut q = world.query_filtered::<&Window, With<PrimaryWindow>>();
    let w = q.single(world).ok()?;
    let (pw, ph) = (w.resolution.physical_width() as f32, w.resolution.physical_height() as f32);
    (pw > 0.0 && ph > 0.0).then_some((pw, ph))
}

/// Replace the UI runtime by a fresh one for the next stage. `setup` registers the game API, runs
/// the stage's scripts and fires the entry events. Returns false when the window has no size yet.
pub fn switch_stage(world: &mut World, setup: impl FnOnce(&mut UiRuntime)) -> bool {
    let Some((w, h)) = window_size(world) else { return false };
    let cfg = world.resource::<UiConfig>().clone();
    let t0 = std::time::Instant::now();
    let mut rt = match UiRuntime::new(&cfg.locale, w, h) {
        Ok(rt) => rt,
        Err(e) => {
            error!("login: cannot create the UI runtime: {e:#}");
            return false;
        }
    };
    if let Some(db) = &cfg.ui_texts_db {
        if let Err(e) = rt.load_ui_texts(db) {
            warn!("login: UI texts from {}: {e:#}; LocalizeUiText returns keys", db.display());
        }
    }
    setup(&mut rt);
    info!("login: UI stage ready in {:.0} ms, {} script errors", t0.elapsed().as_secs_f32() * 1000.0, rt.errors.len());
    for (path, e) in rt.errors.iter().take(10) {
        warn!("login: script {path}: {e}");
    }
    // the old runtime (and its Lua state) drops here
    world.insert_non_send(rt);
    // the new tree starts counting generations again: force a full redraw
    world.resource_mut::<UiRenderState>().generation = u64::MAX;
    set_cameras_active(world, true);
    true
}

/// Leave the UI: an empty runtime, no UI cameras (in game the HUD is not built yet).
pub fn clear(world: &mut World) {
    switch_stage(world, |_| {});
    set_cameras_active(world, false);
}

fn set_cameras_active(world: &mut World, active: bool) {
    let mut q = world.query_filtered::<&mut Camera, With<UiCamera>>();
    for mut c in q.iter_mut(world) {
        c.is_active = active;
    }
}
