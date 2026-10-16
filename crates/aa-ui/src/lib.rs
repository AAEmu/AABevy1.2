//! X2 UI widget toolkit rendered in Bevy and bound to the original Lua UI.
//!
//! The client's UI is Lua (the `.alb` bytecode in `scriptsbin/x2ui`) driving a native widget
//! toolkit. This crate is that toolkit:
//!
//! * [`tree`]: widgets and drawables with anchors, resolved to rectangles in UI units;
//! * [`lua`]: the native API the scripts call (`UIParent:CreateWidget`, `widget:AddAnchor`, ...),
//!   handlers (`SetHandler`) and UI events (`RegisterEvent`, [`UiRuntime::fire_event`]);
//! * [`text`] / [`texture`]: fonts (fonts.g fallback chains, hinted glyphs) and pak textures;
//! * [`render`]: batched textured quads (one mesh per texture run) rebuilt only on change;
//! * [`input`]: mouse hit testing and dispatch.
//!
//! Game API objects (`X2World`, `X2LoginCharacter`, ...) are not part of the toolkit: register
//! them from a [`UiSetup`] hook (see the `world_select` example for a fake set).
//!
//! ```no_run
//! use bevy::prelude::*;
//! App::new()
//!     .add_plugins((DefaultPlugins, aa_ui::UiPlugin::default()))
//!     .insert_resource(aa_ui::UiSetup::new(|ui| {
//!         // register X2World, X2LoginCharacter, ... here, then:
//!         ui.run_stage(aa_ui::Stage::WorldSelect);
//!         ui.fire_event("ENTERED_WORLD_SELECT", ());
//!     }))
//!     .run();
//! ```

pub mod dump;
pub mod input;
pub mod lua;
pub mod render;
pub mod text;
pub mod texture;
pub mod tree;

use bevy::camera::ScalingMode;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

pub use aa_lua::Stage;

use crate::render::{UiBatch, UiCamera, UiRenderState};

/// The Lua state with the toolkit installed. A Bevy non-send resource (Lua is single-threaded).
pub struct UiRuntime {
    pub lua: mlua::Lua,
    mouse: input::MouseState,
    /// Script failures so far (pak path, error).
    pub errors: Vec<(String, String)>,
    init_done: bool,
}

impl UiRuntime {
    /// A fresh Lua state with the toolkit for a screen of `w` x `h` physical pixels.
    pub fn new(locale: &str, w: f32, h: f32) -> anyhow::Result<UiRuntime> {
        let lua = aa_lua::new_lua().map_err(|e| anyhow::anyhow!("{e}"))?;
        let ui_scale = default_ui_scale(w, h);
        let state = lua::Ui {
            tree: tree::Tree::new(w / ui_scale, h / ui_scale),
            fonts: text::FontLib::new(locale),
            textures: texture::TextureDb::new(locale),
            ui_scale,
            handlers: Default::default(),
            events: Default::default(),
            pending: Vec::new(),
            fades: Default::default(),
            time_ms: 0.0,
            frame_ms: 16.0,
            stub_calls: Default::default(),
            hover: None,
            focus: None,
            cursor: (0.0, 0.0),
            queued_events: Vec::new(),
        };
        lua::install(&lua, state).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(UiRuntime {
            lua,
            mouse: Default::default(),
            errors: Vec::new(),
            init_done: false,
        })
    }

    /// Run a `toc.g`; failures are logged and kept in [`Self::errors`].
    pub fn run_toc(&mut self, toc: &str) {
        match aa_lua::run_toc_collect(&self.lua, toc) {
            Ok(errs) => {
                for (path, e) in errs {
                    warn!("aa-ui: script {path}: {e:#}");
                    self.errors.push((path, format!("{e:#}")));
                }
            }
            Err(e) => {
                warn!("aa-ui: toc {toc}: {e:#}");
                self.errors.push((toc.to_string(), format!("{e:#}")));
            }
        }
        lua::flush(&self.lua);
    }

    /// Load a whole screen like the client: the init script (`locale` tables, once per state),
    /// the base contents (`logic`, `baselib`, `components`, `option`), then the screen's contents.
    /// Register the game API objects first (they are called while loading).
    pub fn run_stage(&mut self, stage: Stage) {
        self.run_init_script();
        let files = match stage.scripts() {
            Ok(f) => f,
            Err(e) => {
                warn!("aa-ui: stage {}: {e:#}", stage.name());
                self.errors
                    .push((stage.name().to_string(), format!("{e:#}")));
                return;
            }
        };
        self.run_files(&files);
    }

    /// Only the base contents (`logic`, `baselib`, `components`, `option`) and the init script.
    pub fn load_base_libs(&mut self) {
        self.run_init_script();
        for c in aa_lua::stage::BASE_CONTENTS {
            self.run_toc(&format!("scriptsbin/x2ui/{c}/toc.g"));
        }
    }

    fn run_init_script(&mut self) {
        if !std::mem::replace(&mut self.init_done, true) {
            self.run_files(&[aa_lua::stage::INIT_SCRIPT.to_string()]);
        }
    }

    fn run_files(&mut self, files: &[String]) {
        for (path, e) in aa_lua::run_files(&self.lua, files) {
            warn!("aa-ui: script {path}: {e:#}");
            self.errors.push((path, format!("{e:#}")));
        }
        lua::flush(&self.lua);
    }

    /// Raise a UI event for every widget that registered it.
    pub fn fire_event(&self, name: &str, args: impl mlua::IntoLuaMulti) {
        let args = args
            .into_lua_multi(&self.lua)
            .map(|m| m.into_iter().collect())
            .unwrap_or_default();
        lua::fire_ui_event(&self.lua, name, args);
    }

    /// Raise a UI event at the next frame; for game API callbacks that answer later.
    pub fn queue_event(&self, name: &str, args: impl mlua::IntoLuaMulti) {
        let args = args
            .into_lua_multi(&self.lua)
            .map(|m| m.into_iter().collect())
            .unwrap_or_default();
        lua::queue_ui_event(&self.lua, name, args);
    }

    /// Run a Lua chunk (source text), e.g. to define fake game API objects.
    pub fn exec(&self, name: &str, code: &str) -> mlua::Result<()> {
        self.lua.load(code).set_name(name).exec()
    }

    /// Calls to unimplemented natives so far (`kind:Method` -> count).
    pub fn stub_calls(&self) -> Vec<(String, u32)> {
        lua::ui(&self.lua)
            .stub_calls
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    /// The tree as JSON in the format of `reference/<screen>/ui.json` (see [`dump`]).
    pub fn dump_json(&self) -> String {
        dump::dump_json(&self.lua).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    pub fn ui_scale(&self) -> f32 {
        lua::ui(&self.lua).ui_scale
    }
}

/// The client's default UI scale for a resolution (`GetDefaultUIScaleByResolution`).
pub fn default_ui_scale(w: f32, h: f32) -> f32 {
    if w < 1280.0 || h < 864.0 { 0.85 } else { 1.0 }
}

/// Called once, when the window size is known, with the new runtime: register game objects,
/// load scripts, fire the first events.
#[derive(Resource)]
pub struct UiSetup(pub Box<dyn Fn(&mut UiRuntime) + Send + Sync>);

impl UiSetup {
    pub fn new(f: impl Fn(&mut UiRuntime) + Send + Sync + 'static) -> UiSetup {
        UiSetup(Box::new(f))
    }
}

/// Toolkit settings.
#[derive(Resource, Clone)]
pub struct UiConfig {
    /// `en_us`, `ko`, ...: locale textures, fonts.g section, `X2Locale:GetLocale()`.
    pub locale: String,
    /// UI camera order (draws over cameras with a lower order).
    pub camera_order: isize,
    pub clear: ClearColorConfig,
    /// Blend in gamma (sRGB-encoded) space like the client's Direct3D renderer
    /// (`CompositingSpace::Srgb`); semi-transparent UI looks too light with linear blending.
    /// Cameras stacked on one main texture must agree on the space (Bevy falls back to linear).
    pub gamma_blending: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            locale: "en_us".into(),
            camera_order: 10,
            clear: ClearColorConfig::Default,
            gamma_blending: true,
        }
    }
}

/// Draw / input statistics, updated every frame.
#[derive(Resource, Default, Debug, Clone)]
pub struct UiStats {
    pub quads: usize,
    pub draw_calls: usize,
    pub rebuilds: u64,
}

/// Scripted pointer for tests and automation: while `pos` is set it replaces the OS cursor
/// (physical pixels); `press` / `release` act like a left button edge this frame and are
/// cleared after use.
#[derive(Resource, Default, Debug, Clone)]
pub struct UiVirtualPointer {
    pub pos: Option<Vec2>,
    pub press: bool,
    pub release: bool,
}

#[derive(Default)]
pub struct UiPlugin {
    pub config: Option<UiConfig>,
}

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone().unwrap_or_default())
            .init_resource::<UiRenderState>()
            .init_resource::<UiStats>()
            .init_resource::<UiVirtualPointer>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, (init_runtime, ui_frame).chain());
    }
}

fn spawn_camera(mut commands: Commands, cfg: Res<UiConfig>) {
    let mut cam = commands.spawn((
        Camera2d,
        Camera {
            order: cfg.camera_order,
            clear_color: cfg.clear.clone(),
            ..default()
        },
        UiCamera,
    ));
    if cfg.gamma_blending {
        cam.insert(bevy::camera::CompositingSpace::Srgb);
    }
}

fn window_size(world: &mut World) -> Option<(f32, f32)> {
    let mut q = world.query_filtered::<&Window, With<PrimaryWindow>>();
    let w = q.single(world).ok()?;
    let (pw, ph) = (
        w.resolution.physical_width() as f32,
        w.resolution.physical_height() as f32,
    );
    (pw > 0.0 && ph > 0.0).then_some((pw, ph))
}

fn init_runtime(world: &mut World) {
    if world.get_non_send::<UiRuntime>().is_some() {
        return;
    }
    let Some((w, h)) = window_size(world) else {
        return;
    };
    let locale = world.resource::<UiConfig>().locale.clone();
    let mut rt = match UiRuntime::new(&locale, w, h) {
        Ok(rt) => rt,
        Err(e) => {
            error!("aa-ui: cannot create the UI runtime: {e:#}");
            return;
        }
    };
    let t0 = std::time::Instant::now();
    if let Some(setup) = world.remove_resource::<UiSetup>() {
        (setup.0)(&mut rt);
        world.insert_resource(setup);
    }
    let objects = lua::ui(&rt.lua).tree.objects.len();
    info!(
        "aa-ui: UI ready in {:.0} ms: {objects} objects, {} script errors, ui scale {}",
        t0.elapsed().as_secs_f32() * 1000.0,
        rt.errors.len(),
        rt.ui_scale()
    );
    world.insert_non_send(rt);
}

#[allow(clippy::too_many_arguments)]
fn ui_frame(
    rt: Option<NonSendMut<UiRuntime>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut commands: Commands,
    mut state: ResMut<UiRenderState>,
    mut stats: ResMut<UiStats>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
    formats: Option<Res<bevy::image::CompressedImageFormatSupport>>,
    batches: Query<Entity, With<UiBatch>>,
    mut camera: Query<(&mut Transform, &mut Projection), With<UiCamera>>,
    mut virt: ResMut<UiVirtualPointer>,
) {
    let Some(mut rt) = rt else { return };
    let Ok(window) = windows.single() else { return };
    let (pw, ph) = (
        window.resolution.physical_width() as f32,
        window.resolution.physical_height() as f32,
    );
    if pw <= 0.0 || ph <= 0.0 {
        return;
    }
    let rt = &mut *rt;
    let lua = &rt.lua;
    // Screen size / scale changes.
    let scale = lua::ui(lua).ui_scale;
    let (uw, uh) = (pw / scale, ph / scale);
    let resized = {
        let mut u = lua::ui(lua);
        let changed = (u.tree.screen_w, u.tree.screen_h) != (uw, uh);
        u.tree.set_screen(uw, uh);
        changed
    };
    if resized {
        let ids: Vec<tree::Id> = lua::ui(lua)
            .handlers
            .iter()
            .filter(|(_, h)| h.iter().any(|n| n == "OnScale"))
            .map(|(i, _)| *i)
            .collect();
        for id in ids {
            lua::call_handler(lua, id, "OnScale", vec![]);
        }
    }
    // Input.
    let sf = window.resolution.scale_factor();
    let cursor = match virt.pos {
        Some(p) => Some((p.x / scale, p.y / scale)),
        None => window
            .cursor_position()
            .map(|p| (p.x * sf / scale, p.y * sf / scale)),
    };
    let wheel = scroll.delta.y;
    let (vpress, vrelease) = (
        std::mem::take(&mut virt.press),
        std::mem::take(&mut virt.release),
    );
    input::update(
        lua,
        &mut rt.mouse,
        cursor,
        buttons.just_pressed(MouseButton::Left) || vpress,
        buttons.just_released(MouseButton::Left) || vrelease,
        buttons.just_released(MouseButton::Right),
        wheel,
    );
    // Script time.
    lua::tick(lua, time.delta_secs() * 1000.0);
    // Rebuild the meshes when anything changed.
    let pressed = rt.mouse.pressed;
    let mut g = lua::ui(lua);
    let u = &mut *g;
    if u.tree.generation != state.generation
        || u.fonts.atlas_dirty
        || state.screen != (pw as u32, ph as u32)
    {
        let quads = render::build_quads(u, pressed);
        render::sync_atlas(u, &mut state, &mut images);
        render::sync_batches(
            &mut commands,
            &mut state,
            &quads,
            &mut meshes,
            &mut materials,
            &mut images,
            formats.as_deref(),
            &batches,
        );
        state.generation = u.tree.generation;
        state.screen = (pw as u32, ph as u32);
        stats.quads = state.quads;
        stats.draw_calls = state.draw_calls;
        stats.rebuilds += 1;
    }
    drop(g);
    for (mut tf, mut proj) in camera.iter_mut() {
        *tf = Transform::from_xyz(pw / 2.0, -ph / 2.0, 0.0);
        if let Projection::Orthographic(o) = &mut *proj {
            o.scaling_mode = ScalingMode::Fixed {
                width: pw,
                height: ph,
            };
            o.scale = 1.0;
        }
    }
}
