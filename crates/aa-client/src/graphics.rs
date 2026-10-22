//! The original Options window's screen and quality pages applied to the renderer.
//!
//! The pages write the engine's option ids (`OPTION_ITEM_*`, see `settings`) and `X2Option:Save` counts a save ([`settings::saves`]).
//! The systems here read the saved options again when it changed (and once at startup) and put them on the knobs the world has:
//!
//! | Option | Applied as |
//! |---|---|
//! | screen mode, resolution | the primary window: exclusive fullscreen on the monitor's video mode of that size, else borderless; windowed at that size |
//! | vertical sync | the window's present mode |
//! | gamma | exposure compensation of the world camera's post pass, +-1 stop over the slider |
//! | view distance (draw distance) | `WorldConfig::brush_distance`, `e_max_view_dst` of `option_view_distance.cfg` |
//! | object / vegetation view distance | `view_dist_scale`, `vegetation_distance`, `tree_distance`, the `e_view_dist_ratio*` of `option_view_dist_ratio*.cfg` |
//! | shadows, shadow distance | `WorldConfig::shadows` / `shadow_distance` and the sun's shadow cascades |
//! | clouds | `CloudSettings::enabled` |
//! | effect quality | `FxSettings::quality` |
//! | HDR | the post pass's bloom |
//! | anti-aliasing | the world camera's SMAA preset (the page offers none, PostAA 1x and PostAA 2x) |
//! | camera mode | the world camera's field of view |
//!
//! The rest of the quality page (texture quality, terrain detail distance and level of detail, character and animation level of
//! detail, the shadow level of detail, shader quality, volumetric effects, water quality and reflection, depth of field) is saved
//! and shown but has no knob in the renderer yet. The page's save is the apply: nothing is applied while a slider is dragged.
use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLight};
use bevy::prelude::*;
use bevy::window::{Monitor, MonitorSelection, PresentMode, PrimaryWindow, VideoMode, VideoModeSelection, WindowMode, WindowResized};

use crate::settings;

pub struct GraphicsPlugin;

impl Plugin for GraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<WindowResized>().add_systems(Update, (apply_window, apply_world, apply_camera, apply_sun));
    }
}

/// `OPTION_ITEM_*` ids of the options applied here.
mod id {
    pub const FULLSCREEN: u32 = 0;
    pub const WIDTH: u32 = 1;
    pub const HEIGHT: u32 = 2;
    pub const GAMMA: u32 = 3;
    pub const VSYNC: u32 = 4;
    pub const VIEW_DST: u32 = 10;
    pub const OBJECT_VIEW_DST_RATIO: u32 = 14;
    pub const VEGETATION_VIEW_DST_RATIO: u32 = 15;
    pub const ENABLE_SHADOW: u32 = 17;
    pub const SHADOW_DIST: u32 = 18;
    pub const CLOUDS_EFFECT: u32 = 23;
    pub const EFFECT_QUALITY: u32 = 24;
    pub const HDR: u32 = 27;
    pub const ANTI_ALIASING: u32 = 29;
}

/// The saved options by id (`None`: never saved) and the 1.2 defaults (`settings::native_default`) of the rest.
struct Options<F: Fn(u32) -> Option<f64>>(F);

impl<F: Fn(u32) -> Option<f64>> Options<F> {
    fn saved(&self, id: u32) -> Option<f64> {
        (self.0)(id)
    }

    fn get(&self, id: u32) -> Option<f64> {
        self.saved(id).or_else(|| settings::native_default(id))
    }

    /// A slider or cvar group level, 1 (lowest) to 4, as an index 0 to 3.
    fn level(&self, id: u32) -> usize {
        self.get(id).map_or(3, |v| (v.round() as i64).clamp(1, 4) as usize - 1)
    }

    fn checked(&self, id: u32) -> bool {
        self.get(id).is_some_and(|v| v > 0.0)
    }
}

/// What the Options window set, in the units of the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Graphics {
    /// `None`: never saved, the window stays as it started (the command line's size, `AA_PRESENT`).
    pub fullscreen: Option<bool>,
    pub resolution: Option<(u32, u32)>,
    pub vsync: Option<bool>,
    /// Exposure compensation in stops.
    pub exposure_ev: f32,
    pub bloom: f32,
    pub smaa: Option<Preset>,
    pub shadows: bool,
    pub shadow_distance: f32,
    pub brush_distance: f32,
    pub view_dist_scale: f32,
    pub vegetation_distance: f32,
    pub tree_distance: f32,
    pub clouds: bool,
    pub fx_quality: u8,
}

impl Graphics {
    /// From the saved options (an edit the window has not saved yet does not count).
    pub fn read() -> Self {
        Self::from_options(|id| settings::option_number(&format!("n:{id}")))
    }

    /// From a source of saved option values by `OPTION_ITEM_*` id.
    fn from_options(saved: impl Fn(u32) -> Option<f64>) -> Self {
        let o = Options(saved);
        let vegetation = [30.0, 40.0, 50.0, 60.0][o.level(id::VEGETATION_VIEW_DST_RATIO)];
        Graphics {
            fullscreen: o.saved(id::FULLSCREEN).map(|v| v > 0.0),
            resolution: o.saved(id::WIDTH).zip(o.saved(id::HEIGHT)).filter(|(w, h)| *w >= 640.0 && *h >= 480.0).map(|(w, h)| (w as u32, h as u32)),
            vsync: o.saved(id::VSYNC).map(|v| v > 0.0),
            exposure_ev: o.get(id::GAMMA).map_or(0.0, |g| (g as f32 - 1.0).clamp(-0.5, 0.5) * 2.0),
            bloom: if o.checked(id::HDR) { 1.0 } else { 0.0 },
            smaa: smaa(o.get(id::ANTI_ALIASING).unwrap_or(3.0)),
            shadows: o.checked(id::ENABLE_SHADOW),
            shadow_distance: [100.0, 150.0, 200.0, 300.0][o.level(id::SHADOW_DIST)],
            // `e_max_view_dst`
            brush_distance: [600.0, 1000.0, 1500.0, 2000.0][o.level(id::VIEW_DST)],
            // the placement files were exported with `e_view_dist_ratio` 40, so the scale is the ratio over 40 (60 is the default, 1.5)
            view_dist_scale: [30.0, 30.0, 40.0, 60.0][o.level(id::OBJECT_VIEW_DST_RATIO)] / 40.0,
            // the caps are the default ones (level 4, ratio 60) scaled by `e_view_dist_ratio_vegetation`
            vegetation_distance: 220.0 * vegetation / 60.0,
            tree_distance: 1000.0 * vegetation / 60.0,
            clouds: o.checked(id::CLOUDS_EFFECT) && std::env::var_os("AA_NO_CLOUDS").is_none(),
            // the particle data's `ConfigMin`/`ConfigMax` level, 0 to 3: the default, Very High, is the High (2) the effects were
            // fitted at (`FxSettings::default`), so a lower level steps down from there
            fx_quality: (o.level(id::EFFECT_QUALITY) as u8).saturating_sub(1),
        }
    }
}

/// The post pass's anti-aliasing quality (SMAA).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Medium,
    High,
}

impl From<Preset> for SmaaPreset {
    fn from(p: Preset) -> Self {
        match p {
            Preset::Medium => SmaaPreset::Medium,
            Preset::High => SmaaPreset::High,
        }
    }
}

/// `OPTION_ITEM_ANTI_ALIASING`: 1 is "not use", 2 "PostAA 1x", 3 "PostAA 2x" and 4 to 12 the MSAA and TXAA entries the page lists only
/// for formats the device enumerates (`EnumAAFormats` returns none).
fn smaa(value: f64) -> Option<Preset> {
    match value.round() as i64 {
        ..=1 => None,
        2 => Some(Preset::Medium),
        _ => Some(Preset::High),
    }
}

/// The exclusive fullscreen video mode of a resolution: the one with the highest refresh rate and bit depth.
fn video_mode(modes: &[VideoMode], (w, h): (u32, u32)) -> Option<VideoMode> {
    modes.iter().filter(|m| m.physical_size == UVec2::new(w, h)).max_by_key(|m| (m.refresh_rate_millihertz, m.bit_depth)).copied()
}

/// The window mode of the screen mode option: fullscreen is exclusive on the video mode of the chosen size, borderless when the
/// monitor has none of that size; no saved option keeps the window's mode.
fn window_mode(fullscreen: Option<bool>, size: (u32, u32), modes: &[VideoMode], current: WindowMode) -> WindowMode {
    match fullscreen {
        Some(true) => match video_mode(modes, size) {
            Some(m) => WindowMode::Fullscreen(MonitorSelection::Current, VideoModeSelection::Specific(m)),
            None => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        },
        Some(false) => WindowMode::Windowed,
        None => current,
    }
}

/// The resolutions offered: those of the monitor's video modes from 1024x720 up, ascending, and `current` (the window's own size).
fn resolutions(modes: &[VideoMode], current: (u32, u32)) -> Vec<(u32, u32)> {
    let mut list: Vec<_> = modes.iter().map(|m| (m.physical_size.x, m.physical_size.y)).filter(|(w, h)| *w >= 1024 && *h >= 720).collect();
    list.push(current);
    list.sort_unstable();
    list.dedup();
    list
}

/// `Some(seen)` has applied the options of that save count; the first run (and each save after) applies.
fn fresh(seen: &mut Option<u32>) -> bool {
    let now = settings::saves();
    let changed = *seen != Some(now);
    *seen = Some(now);
    changed
}

/// Window mode, size and present mode. Only what was saved is applied, and at startup not over what the command line chose
/// (`--size`, `AA_PRESENT`); a window with no OS window (`--headless`) is left alone.
fn apply_window(
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut resized: MessageWriter<WindowResized>,
    monitors: Query<&Monitor>,
    args: Option<Res<aa_dev::DevArgs>>,
    mut seen: Local<Option<u32>>,
    mut first: Local<bool>,
    mut listed: Local<Option<Vec<(u32, u32)>>>,
) {
    let Ok((entity, mut window)) = windows.single_mut() else { return };
    // the list the page shows: the video modes of the monitor the window is on (the first one before it has any), and the window's size
    let modes: Vec<VideoMode> = monitors.iter().next().map(|m| m.video_modes.clone()).unwrap_or_default();
    let current = (window.physical_width(), window.physical_height());
    let list = resolutions(&modes, current);
    if listed.as_ref() != Some(&list) {
        settings::set_resolutions(list.clone());
        *listed = Some(list);
    }
    if !fresh(&mut seen) || args.is_some_and(|a| a.headless) {
        return;
    }
    let startup = !std::mem::replace(&mut *first, true);
    let g = Graphics::read();
    let sized = std::env::args().any(|a| a == "--size");
    if let Some(vsync) = g.vsync
        && !(startup && std::env::var_os("AA_PRESENT").is_some())
    {
        let mode = if vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
        if window.present_mode != mode {
            window.present_mode = mode;
            info!("graphics: present mode {mode:?}");
        }
    }
    if startup && sized {
        return;
    }
    let size = g.resolution.unwrap_or(current);
    let mode = window_mode(g.fullscreen, size, &modes, window.mode);
    if window.mode != mode {
        window.mode = mode;
        info!("graphics: window mode {mode:?}");
    }
    if matches!(mode, WindowMode::Windowed) && g.resolution.is_some() && size != current {
        window.resolution.set_physical_resolution(size.0, size.1);
        info!("graphics: window {}x{}", size.0, size.1);
        // the cameras take their target size from the message (winit sends its own once the OS has resized the window): without it
        // the passes of the frame in between disagree about the size
        resized.write(WindowResized { window: entity, width: window.width(), height: window.height() });
    }
}

/// The streaming distances, shadows, clouds and effects, once the world exists and again when the options were saved; held back while
/// the prewarm has the streaming settings (it puts its own back when done).
#[allow(clippy::too_many_arguments)]
fn apply_world(
    cfg: Option<ResMut<aa_world::WorldConfig>>,
    clouds: Option<ResMut<aa_world::sky_clouds::CloudSettings>>,
    fx: Option<ResMut<aa_fx::FxSettings>>,
    prewarm: Option<Res<crate::ingame::prewarm::Prewarm>>,
    mut seen: Local<Option<u32>>,
    mut pending: Local<bool>,
) {
    *pending |= fresh(&mut seen) || cfg.as_ref().is_some_and(|c| c.is_added());
    if !*pending || prewarm.is_some_and(|p| p.running()) {
        return;
    }
    *pending = false;
    let g = Graphics::read();
    // the changes are flagged only when a value moved: the sun rebuilds its cascades on a changed config
    if let Some(mut cfg) = cfg
        && g.apply_world(cfg.bypass_change_detection())
    {
        cfg.set_changed();
        info!("graphics: view distance {} m, objects x{}, shadows {} to {} m", g.brush_distance, g.view_dist_scale, g.shadows, g.shadow_distance);
    }
    if let Some(mut clouds) = clouds
        && clouds.enabled != g.clouds
    {
        clouds.enabled = g.clouds;
    }
    if let Some(mut fx) = fx
        && fx.quality != g.fx_quality
    {
        fx.quality = g.fx_quality;
    }
}

/// Assigns `value` and says whether it was another one.
fn set<T: PartialEq>(slot: &mut T, value: T) -> bool {
    let moved = *slot != value;
    *slot = value;
    moved
}

impl Graphics {
    /// The distances and shadows of the world's streaming settings; true when one changed.
    pub fn apply_world(&self, cfg: &mut aa_world::WorldConfig) -> bool {
        let mut moved = false;
        moved |= set(&mut cfg.brush_distance, self.brush_distance);
        moved |= set(&mut cfg.view_dist_scale, self.view_dist_scale);
        moved |= set(&mut cfg.vegetation_distance, self.vegetation_distance);
        moved |= set(&mut cfg.tree_distance, self.tree_distance);
        moved |= set(&mut cfg.shadows, self.shadows);
        moved |= set(&mut cfg.shadow_distance, self.shadow_distance);
        moved
    }
}

/// Bloom, exposure and anti-aliasing of the world camera (the one with the post pass), when it appears and after a save; the camera
/// mode's field of view.
fn apply_camera(
    mut commands: Commands,
    mut cameras: Query<(Entity, &mut aa_world::post::Post, Option<&mut Smaa>)>,
    fov: Option<ResMut<crate::ingame::cam_fov::CamFov>>,
    mut seen: Local<Option<u32>>,
) {
    let saved = fresh(&mut seen);
    if saved && let Some(mut fov) = fov {
        fov.rewrite();
    }
    let mut g = None;
    for (entity, mut post, smaa) in &mut cameras {
        if !saved && !post.is_added() {
            continue;
        }
        let g: &Graphics = g.get_or_insert_with(Graphics::read);
        post.bloom = g.bloom;
        post.exposure_ev = g.exposure_ev;
        match (smaa, g.smaa) {
            (Some(mut s), Some(preset)) => s.preset = preset.into(),
            (None, Some(preset)) => {
                commands.entity(entity).insert(Smaa { preset: preset.into() });
            }
            (Some(_), None) => {
                commands.entity(entity).remove::<Smaa>();
            }
            (None, None) => {}
        }
    }
}

/// The sun's shadow switch and cascades follow `WorldConfig` (aa-world builds them once, when the sun is spawned).
fn apply_sun(cfg: Option<Res<aa_world::WorldConfig>>, mut sun: Query<(&mut DirectionalLight, &mut CascadeShadowConfig), With<aa_world::sky::SunLight>>) {
    let Some(cfg) = cfg.filter(|c| c.is_changed()) else { return };
    for (mut light, mut cascades) in &mut sun {
        light.shadow_maps_enabled = cfg.shadows;
        *cascades = CascadeShadowConfigBuilder {
            num_cascades: 3,
            minimum_distance: 0.3,
            maximum_distance: cfg.shadow_distance,
            first_cascade_far_bound: cfg.shadow_distance * 0.08,
            ..default()
        }
        .build();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(saved: &[(u32, f64)]) -> Graphics {
        let saved = saved.to_vec();
        Graphics::from_options(move |id| saved.iter().find(|(i, _)| *i == id).map(|(_, v)| *v))
    }

    /// Nothing saved is the 1.2 default (every level 4, shadows, clouds and HDR on): exactly the settings the world and the effects
    /// were calibrated at, so the options window changes nothing until the player does.
    #[test]
    fn defaults_are_what_the_world_was_calibrated_at() {
        let g = with(&[]);
        let cfg = aa_world::WorldConfig::new("main_world");
        let mut applied = cfg.clone();
        assert!(!g.apply_world(&mut applied), "the default options leave the world's configuration alone");
        assert_eq!(
            (applied.brush_distance, applied.view_dist_scale, applied.vegetation_distance, applied.tree_distance, applied.shadows, applied.shadow_distance),
            (cfg.brush_distance, cfg.view_dist_scale, cfg.vegetation_distance, cfg.tree_distance, cfg.shadows, cfg.shadow_distance)
        );
        assert_eq!(g.fx_quality, aa_fx::FxSettings::default().quality);
        assert!(g.clouds == std::env::var_os("AA_NO_CLOUDS").is_none());
        assert_eq!((g.exposure_ev, g.bloom, g.smaa), (0.0, 1.0, Some(Preset::High)));
        assert_eq!((g.fullscreen, g.resolution, g.vsync), (None, None, None), "the window stays as it started");
    }

    #[test]
    fn levels_map_to_the_cvar_groups() {
        let low = with(&[(id::VIEW_DST, 1.0), (id::OBJECT_VIEW_DST_RATIO, 1.0), (id::VEGETATION_VIEW_DST_RATIO, 1.0), (id::SHADOW_DIST, 1.0), (id::EFFECT_QUALITY, 1.0)]);
        // option_view_distance.cfg [1], option_view_dist_ratio.cfg [1] (30 of the 40 the placements were exported with), vegetation [1]
        assert_eq!((low.brush_distance, low.view_dist_scale), (600.0, 0.75));
        assert_eq!((low.vegetation_distance, low.tree_distance), (110.0, 500.0));
        assert_eq!((low.shadow_distance, low.fx_quality), (100.0, 0));
        let mid = with(&[(id::VIEW_DST, 3.0), (id::OBJECT_VIEW_DST_RATIO, 3.0), (id::VEGETATION_VIEW_DST_RATIO, 3.0), (id::EFFECT_QUALITY, 3.0)]);
        assert_eq!((mid.brush_distance, mid.view_dist_scale, mid.fx_quality), (1500.0, 1.0, 1));
        assert!((mid.vegetation_distance - 183.33).abs() < 0.01);
        // a value off the slider is clamped, not an index error
        assert_eq!(with(&[(id::VIEW_DST, 9.0)]).brush_distance, 2000.0);
        assert_eq!(with(&[(id::VIEW_DST, 0.0)]).brush_distance, 600.0);
    }

    #[test]
    fn checkboxes_gamma_and_anti_aliasing() {
        let g = with(&[(id::ENABLE_SHADOW, 0.0), (id::CLOUDS_EFFECT, 0.0), (id::HDR, 0.0), (id::GAMMA, 1.5), (id::ANTI_ALIASING, 1.0)]);
        assert!(!g.shadows && !g.clouds);
        assert_eq!((g.bloom, g.exposure_ev, g.smaa), (0.0, 1.0, None));
        assert_eq!(with(&[(id::GAMMA, 0.5)]).exposure_ev, -1.0);
        assert_eq!(with(&[(id::GAMMA, 9.0)]).exposure_ev, 1.0, "a gamma off the slider is clamped");
        assert_eq!(with(&[(id::ANTI_ALIASING, 2.0)]).smaa, Some(Preset::Medium));
        // the MSAA and TXAA entries are not offered, a file that has one gets the better SMAA
        assert_eq!(with(&[(id::ANTI_ALIASING, 5.0)]).smaa, Some(Preset::High));
    }

    #[test]
    fn world_config_changes_are_reported_once() {
        let mut cfg = aa_world::WorldConfig::new("main_world");
        let g = with(&[(id::VIEW_DST, 2.0), (id::ENABLE_SHADOW, 0.0)]);
        assert!(g.apply_world(&mut cfg));
        assert_eq!((cfg.brush_distance, cfg.shadows), (1000.0, false));
        assert!(!g.apply_world(&mut cfg), "the same options again move nothing");
    }

    #[test]
    fn window_settings_need_a_saved_option() {
        let mode = |w, h, hz| VideoMode { physical_size: UVec2::new(w, h), bit_depth: 32, refresh_rate_millihertz: hz };
        let modes = [mode(1920, 1080, 60_000), mode(1920, 1080, 144_000), mode(1280, 720, 60_000), mode(800, 600, 60_000)];
        let g = with(&[(id::FULLSCREEN, 1.0), (id::WIDTH, 1920.0), (id::HEIGHT, 1080.0), (id::VSYNC, 1.0)]);
        assert_eq!((g.fullscreen, g.resolution, g.vsync), (Some(true), Some((1920, 1080)), Some(true)));
        // exclusive fullscreen takes the fastest mode of that size; a size the monitor lacks falls back to borderless
        let WindowMode::Fullscreen(_, VideoModeSelection::Specific(m)) = window_mode(Some(true), (1920, 1080), &modes, WindowMode::Windowed) else {
            panic!("exclusive fullscreen")
        };
        assert_eq!(m.refresh_rate_millihertz, 144_000);
        assert!(matches!(window_mode(Some(true), (1600, 900), &modes, WindowMode::Windowed), WindowMode::BorderlessFullscreen(_)));
        assert_eq!(window_mode(Some(false), (1920, 1080), &modes, WindowMode::BorderlessFullscreen(MonitorSelection::Current)), WindowMode::Windowed);
        assert_eq!(window_mode(None, (1920, 1080), &modes, WindowMode::Windowed), WindowMode::Windowed);
        // a resolution of the page's list below the smallest one is not taken for a saved size
        assert_eq!(with(&[(id::WIDTH, 320.0), (id::HEIGHT, 200.0)]).resolution, None);
        // the page's list: the monitor's sizes from 1024x720 up, once each, and the window's own
        assert_eq!(resolutions(&modes, (1500, 900)), vec![(1280, 720), (1500, 900), (1920, 1080)]);
        assert_eq!(resolutions(&[], (1920, 1080)), vec![(1920, 1080)]);
    }

    #[test]
    fn saves_are_seen_once() {
        let mut seen = None;
        assert!(fresh(&mut seen), "the first look applies");
        assert!(!fresh(&mut seen));
    }

    /// The plugin's systems run together (Bevy reports conflicting access when they are initialised). What they set depends on the
    /// account's saved options, which the tests above cover with injected ones.
    #[test]
    fn the_systems_run_together() {
        let mut app = App::new();
        app.add_plugins(GraphicsPlugin)
            .insert_resource(aa_world::WorldConfig::new("main_world"))
            .insert_resource(aa_world::sky_clouds::CloudSettings { enabled: true })
            .insert_resource(aa_fx::FxSettings::default());
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let camera = app.world_mut().spawn((aa_world::post::Post::default(), Smaa::default())).id();
        app.world_mut().spawn((aa_world::sky::SunLight, DirectionalLight::default(), CascadeShadowConfigBuilder::default().build()));
        app.update();
        app.update();
        assert!(app.world().get::<aa_world::post::Post>(camera).is_some());
    }

    /// The sun follows a changed `WorldConfig`: the shadow switch and the cascade distance.
    #[test]
    fn the_sun_follows_the_world_config() {
        let mut app = App::new();
        let mut cfg = aa_world::WorldConfig::new("main_world");
        cfg.shadows = false;
        cfg.shadow_distance = 150.0;
        app.insert_resource(cfg).add_systems(Update, apply_sun);
        let sun = app.world_mut().spawn((aa_world::sky::SunLight, DirectionalLight { shadow_maps_enabled: true, ..default() }, CascadeShadowConfigBuilder::default().build())).id();
        app.update();
        let light = app.world().get::<DirectionalLight>(sun).unwrap();
        assert!(!light.shadow_maps_enabled);
        let cascades = app.world().get::<CascadeShadowConfig>(sun).unwrap();
        assert_eq!(*cascades.bounds.last().unwrap(), 150.0);
        app.world_mut().resource_mut::<aa_world::WorldConfig>().shadows = true;
        app.update();
        assert!(app.world().get::<DirectionalLight>(sun).unwrap().shadow_maps_enabled);
    }
}
