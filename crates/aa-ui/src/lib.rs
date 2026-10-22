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
//! * [`input`]: mouse hit testing and dispatch; keyboard focus and typing for edit boxes
//!   ([`lua::edit`], [`UiRuntime::key`]).
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

pub mod content;
pub mod cursor;
pub mod dump;
pub mod input;
pub mod item_link;
pub mod lua;
pub mod prof;
pub mod render;
pub mod text;
pub mod texture;
pub mod tree;
pub mod uitext;

use bevy::camera::ScalingMode;
use bevy::input::keyboard::{KeyboardFocusLost, KeyboardInput};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseButtonInput};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

pub use aa_lua::Stage;
pub use lua::edit::UiKey;
pub use lua::keyboard::{Modifiers, native_key_name};

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
        cursor::init(&lua);
        let ui_scale = default_ui_scale(w, h);
        let state = lua::Ui {
            tree: tree::Tree::new(w / ui_scale, h / ui_scale),
            fonts: text::FontLib::new(locale),
            textures: texture::TextureDb::new(locale),
            ui_scale,
            handlers: Default::default(),
            frame_handlers: Default::default(),
            events: Default::default(),
            pending: Vec::new(),
            retiring: Default::default(),
            fades: Default::default(),
            fade_clock: 0.0,
            next_fade_order: 0,
            animations: lua::animation::Animations::with_ui_scale(ui_scale),
            damage: Default::default(),
            time_ms: 0.0,
            frame_ms: 16.0,
            cooldown_captions: Default::default(),
            auto_clip_widgets: Default::default(),
            stub_calls: Default::default(),
            hover: None,
            entered: Vec::new(),
            focus: None,
            keyboard: Default::default(),
            clipboard: String::new(),
            caret_visible: false,
            caret_t0: 0.0,
            cursor: (0.0, 0.0),
            queued_events: Vec::new(),
            widget_sounds: Default::default(),
            drag: Default::default(),
            attachments: Default::default(),
        };
        lua::install(&lua, state).map_err(|e| anyhow::anyhow!("{e}"))?;
        prof::wrap_natives(&lua).map_err(|e| anyhow::anyhow!("{e}"))?;
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

    /// Deliver a key press to the widget with the keyboard focus: edit boxes insert text and
    /// fire `OnTextChanged`, Enter / Escape / Tab fire `OnEnterPressed` / `OnEscapePressed` /
    /// `OnTabPressed`. Returns false when no widget has the focus (the key is the game's).
    pub fn key(&self, k: &UiKey) -> bool {
        lua::edit::key(&self.lua, k)
    }

    /// Type text into the focused edit box (see [`Self::key`]).
    pub fn type_text(&self, s: &str) -> bool {
        self.key(&UiKey::Text(s.to_string()))
    }

    /// The visible keyboard capture owner, or the focused widget (including navigation focus).
    /// Use [`Self::keyboard_captured`] to decide whether gameplay keys are suppressed.
    pub fn focus(&self) -> Option<tree::Id> {
        let u = lua::ui(&self.lua);
        u.keyboard.owner(&u.tree, u.focus)
    }

    /// True while UI owns keyboard input, including the frame capture ends and held UI keys.
    /// Gameplay input should check this after [`UiInput`].
    pub fn keyboard_captured(&self) -> bool {
        let u = lua::ui(&self.lua);
        u.keyboard.captured(&u.tree, u.focus)
    }

    pub fn modifiers(&self) -> Modifiers {
        lua::ui(&self.lua).keyboard.modifiers()
    }

    /// Forward an OS mouse button edge as a key (`mouse3`, `mouse4`, `mouse5`): only a key capture takes it, see `lua::keyboard::button_event`.
    pub fn button_event(&self, name: &str, pressed: bool) -> bool {
        lua::keyboard::button_event(&self.lua, name, pressed)
    }

    /// Forward an OS key edge. Returns true if UI consumed it.
    pub fn key_event(&self, code: KeyCode, pressed: bool, text: Option<&str>) -> bool {
        lua::keyboard::event(&self.lua, code, pressed, text)
    }

    /// Deliver an OS IME event to its enabled edit owner. Composition is visible to
    /// original GetText/OnTextChanged handlers; it does not invent physical key edges.
    pub fn ime_event(&self, event: &bevy::window::Ime) -> bool {
        lua::edit::ime::event(&self.lua, event)
    }

    /// Load the game database's UI texts (cached per path and locale) and serve
    /// `X2Locale:LocalizeUiText` from them. Returns the number of texts.
    pub fn load_ui_texts(&self, db: &std::path::Path) -> anyhow::Result<usize> {
        let locale = lua::ui(&self.lua).textures.locale.clone();
        let texts = uitext::load(db, &locale)?;
        let n = texts.len();
        self.set_ui_texts(texts)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(n)
    }

    /// Serve `X2Locale:LocalizeUiText(category, key, ...)` from `texts` (`$1`.. are replaced by
    /// the extra arguments, a missing text returns the key).
    pub fn set_ui_texts(&self, texts: std::sync::Arc<uitext::UiTexts>) -> mlua::Result<()> {
        lua::slot::set_caption_texts(&self.lua, &texts);
        let x2locale = lua::install_game_stub(&self.lua, "X2Locale")?;
        let f = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let mut it = args.into_iter().skip(1); // self
                let cat = match it.next() {
                    Some(mlua::Value::Integer(i)) => i,
                    Some(mlua::Value::Number(n)) => n as i64,
                    _ => 0,
                };
                let key = match it.next() {
                    Some(mlua::Value::String(s)) => s.to_string_lossy(),
                    Some(v) => lua_tostring(lua, v),
                    None => String::new(),
                };
                let rest: Vec<String> = it.map(|v| lua_tostring(lua, v)).collect();
                Ok(uitext::localize(&texts, cat, &key, &rest))
            })?;
        x2locale.raw_set("LocalizeUiText", f)
    }

    /// Ask `AAUI.SlotContent` / `SkillContent` / `ItemContent` again for every bound slot (call
    /// it when action bars, bags or skills change) and tell the scripts (`OnContentUpdated`).
    pub fn refresh_slots(&self) -> mlua::Result<()> {
        lua::slot::refresh_slots(&self.lua)
    }

    /// Toggle an in-game window by its `ADDON` content id (`UIC_BAG`, ... see
    /// [`Self::content_id`]), like the main menu buttons: `ADDON:ToggleContent(content)`.
    pub fn toggle_content(&self, content: i64) -> mlua::Result<()> {
        content::toggle(&self.lua, content)
    }

    /// Show or hide an in-game window by content id: `ADDON:ShowContent(content, show)`.
    pub fn show_content(&self, content: i64, show: bool) -> mlua::Result<()> {
        content::show(&self.lua, content, show)
    }

    /// Whether a content's registered widget is shown (contents registered only with a trigger
    /// function, such as `UIC_SKILL`, report false).
    pub fn content_visible(&self, content: i64) -> bool {
        content::visible(&self.lua, content)
    }

    /// The value of a content constant by name (`"UIC_BAG"` -> 19).
    pub fn content_id(&self, name: &str) -> Option<i64> {
        content::content_id(&self.lua, name)
    }

    /// Run a key binding action of the client (`toggle_bag`, `toggle_character`,
    /// `toggle_spellbook`, `toggle_quest`, `toggle_worldmap`, `open_config`, ...): toggles the
    /// content the main menu table binds to it. Returns false for actions that are not UI
    /// windows (movement, targeting: the app's).
    pub fn hotkey(&self, action: &str) -> mlua::Result<bool> {
        content::hotkey(&self.lua, action)
    }

    /// Escape outside an edit box: close the topmost window that closes on Escape, or open the
    /// system menu. Call it when [`Self::key`] did not take the Escape key.
    pub fn escape(&self) -> mlua::Result<content::Escaped> {
        content::escape(&self.lua)
    }

    /// Give the keyboard focus to a widget (`None` clears it), like `widget:SetFocus()`.
    pub fn set_focus(&self, id: Option<tree::Id>) {
        lua::edit::set_focus(&self.lua, id);
    }

    /// Whether the pointer holds something that can be dropped (a picked up item or skill, see
    /// [`lua::drag`]); a window's move cursor is not.
    pub fn holding(&self) -> bool {
        lua::drag::holding(&self.lua)
    }

    /// Let go of what the pointer holds (`X2Cursor:ClearCursor()`).
    pub fn clear_cursor(&self) {
        lua::drag::clear(&self.lua);
    }

    /// The held thing was dropped where no widget takes it (a click on the game world): `AAUI.DropOnNothing(overUi)`.
    pub fn drop_on_nothing(&self, over_ui: bool) {
        if self.holding() {
            lua::drag::drop_on_nothing(&self.lua, over_ui);
        }
    }
}

/// `tostring(v)` as Lua prints it (whole numbers without a fraction).
fn lua_tostring(lua: &mlua::Lua, v: mlua::Value) -> String {
    match v {
        mlua::Value::String(s) => s.to_string_lossy(),
        mlua::Value::Integer(i) => i.to_string(),
        mlua::Value::Number(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", n as i64),
        mlua::Value::Number(n) => n.to_string(),
        other => lua
            .globals()
            .get::<mlua::Function>("tostring")
            .and_then(|f| f.call::<String>(other))
            .unwrap_or_default(),
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

/// Command: throw the current UI away and build a fresh one with `setup`, like the client
/// does on every stage change (world select -> character select -> loading -> in game: each
/// stage gets a new Lua state). The old Lua state, its widgets and handlers are dropped; the
/// new runtime is created by the plugin at the next update, so `setup` runs exactly as an
/// initial [`UiSetup`] would.
///
/// ```no_run
/// # use bevy::prelude::*;
/// fn enter_character_select(mut commands: Commands) {
///     commands.queue(aa_ui::ReloadUi::new(|ui| {
///         // register X2* objects ...
///         ui.run_stage(aa_ui::Stage::CharacterSelect);
///         ui.fire_event("SHOW_CHARACTER_SELECT_WINDOW", true);
///     }));
/// }
/// ```
pub struct ReloadUi(pub UiSetup);

impl ReloadUi {
    pub fn new(f: impl Fn(&mut UiRuntime) + Send + Sync + 'static) -> ReloadUi {
        ReloadUi(UiSetup::new(f))
    }
}

impl Command for ReloadUi {
    type Out = ();
    fn apply(self, world: &mut World) {
        reload_ui(world, self.0);
    }
}

/// [`ReloadUi`] for exclusive systems: drop the current [`UiRuntime`] and install `setup` for
/// the next one.
pub fn reload_ui(world: &mut World, setup: UiSetup) {
    let had = world.remove_non_send::<UiRuntime>().is_some();
    world.insert_resource(setup);
    // The new tree's generation may equal the last one drawn: force a rebuild.
    if let Some(mut st) = world.get_resource_mut::<UiRenderState>() {
        st.generation = u64::MAX;
    }
    if let Some(mut stats) = world.get_resource_mut::<UiStats>() {
        stats.keyboard_focus = false;
    }
    info!(
        "aa-ui: reloading the UI (previous runtime {})",
        if had { "dropped" } else { "none" }
    );
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
    /// Split mode for login stages with a 3D scene: windows on the `background` UI layer (the
    /// painted login backgrounds, `SetUILayer("background")`) are drawn by a second camera at
    /// this order, below the app's 3D camera; everything else by the UI camera at
    /// `camera_order`, above it. The background camera clears with `clear`, the UI camera then
    /// loads (`ClearColorConfig::None`), so the 3D camera between them must not clear colour
    /// either. `None` (default): one camera draws the whole UI.
    pub background_camera_order: Option<isize>,
    /// Render layer of the background camera and its batches in split mode (the UI camera and
    /// its batches stay on layer 0, which 2D cameras of the app also see).
    pub background_render_layer: usize,
    /// Game database with `ui_texts` / `localized_texts` (e.g. `compact.sqlite3`): when set,
    /// `X2Locale:LocalizeUiText(category, key, ...)` returns the localized text with `$1`..
    /// substituted, loaded once per path and locale. Unset: the key.
    pub ui_texts_db: Option<std::path::PathBuf>,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            locale: "en_us".into(),
            camera_order: 10,
            clear: ClearColorConfig::Default,
            gamma_blending: true,
            background_camera_order: None,
            background_render_layer: 31,
            ui_texts_db: None,
        }
    }
}

/// Draw / input statistics, updated every frame.
#[derive(Resource, Default, Debug, Clone)]
pub struct UiStats {
    pub quads: usize,
    pub draw_calls: usize,
    pub rebuilds: u64,
    /// Batch meshes uploaded since start (a rebuild re-uploads only the batches that changed).
    pub mesh_uploads: usize,
    /// A widget has keyboard capture/edit focus, or UI consumed a key this frame: the app should not treat keys as game
    /// bindings this frame.
    pub keyboard_focus: bool,
    /// CPU time of the last UI frame (input, Lua tick, rebuild if any), microseconds.
    pub frame_us: f32,
}

/// Scripted pointer for tests and automation: while `pos` is set it replaces the OS cursor
/// (physical pixels); `press` / `release` act like a left button edge this frame and are
/// cleared after use. `wheel` is mouse wheel notches for this frame (> 0 = up, scrolls chat
/// back), added to the OS wheel and cleared after use.
#[derive(Resource, Default, Debug, Clone)]
pub struct UiVirtualPointer {
    pub pos: Option<Vec2>,
    pub press: bool,
    pub release: bool,
    /// A right button press, cleared after use; dispatches native outside-click listeners.
    pub right_press: bool,
    /// A right button click (the release counts: right clicks fire on release), cleared after use.
    pub right_release: bool,
    pub wheel: f32,
}

#[derive(Default)]
pub struct UiPlugin {
    pub config: Option<UiConfig>,
}

/// UI key delivery and consumption handoff. Gameplay keyboard systems should run after this set.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiInput;

/// Pointer input, Lua/native animation sampling, mesh refresh and held-cursor refresh.
/// Applications can update projected unit positions after gameplay/camera movement and
/// `.before(UiFrame)` so combat text uses this frame's camera. Runs after [`UiInput`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiFrame;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        render::install_material(app);
        app.insert_resource(self.config.clone().unwrap_or_default())
            .init_resource::<cursor::CursorSource>()
            .init_resource::<UiRenderState>()
            .init_resource::<UiStats>()
            .init_resource::<UiVirtualPointer>()
            .add_message::<bevy::window::Ime>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, (init_runtime, ui_keyboard.in_set(UiInput), ui_frame.in_set(UiFrame), ui_cursor.in_set(UiFrame), cursor::sync.in_set(UiFrame), ui_ime_window.in_set(UiFrame)).chain());
    }
}

fn spawn_camera(mut commands: Commands, cfg: Res<UiConfig>, mut state: ResMut<UiRenderState>) {
    // A stack that holds a Camera3d composites in linear space (Bevy's 3D path writes linear
    // values), and cameras that disagree get separate main textures, so the later one would
    // overwrite the 3D layer: in split mode the UI cameras blend in linear space too.
    let space = if cfg.gamma_blending && cfg.background_camera_order.is_none() {
        bevy::camera::CompositingSpace::Srgb
    } else {
        bevy::camera::CompositingSpace::Linear
    };
    // Split mode: the background camera clears, the UI camera draws over whatever is below.
    let ui_clear = match cfg.background_camera_order {
        Some(order) => {
            commands.spawn((
                Camera2d,
                Camera {
                    order,
                    clear_color: cfg.clear,
                    ..default()
                },
                bevy::camera::visibility::RenderLayers::layer(cfg.background_render_layer),
                space,
                UiCamera,
                UiBackgroundCamera,
            ));
            state.background_layer = Some(cfg.background_render_layer);
            ClearColorConfig::None
        }
        None => cfg.clear,
    };
    commands.spawn((
        Camera2d,
        Camera {
            order: cfg.camera_order,
            clear_color: ui_clear,
            ..default()
        },
        space,
        UiCamera,
    ));
}

/// Marker of the background camera in split mode (`UiConfig::background_camera_order`).
#[derive(Component)]
pub struct UiBackgroundCamera;

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
    if let Some(db) = world.resource::<UiConfig>().ui_texts_db.clone() {
        match rt.load_ui_texts(&db) {
            Ok(n) => info!("aa-ui: {n} UI texts from {}", db.display()),
            Err(e) => warn!(
                "aa-ui: UI texts from {}: {e:#}; LocalizeUiText returns keys",
                db.display()
            ),
        }
    }
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

/// The icon of what the pointer holds ([`lua::drag`]) follows the pointer, drawn over everything.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn ui_cursor(
    rt: Option<NonSendMut<UiRuntime>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    virt: Res<UiVirtualPointer>,
    mut commands: Commands,
    mut state: ResMut<UiRenderState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut mask_materials: ResMut<Assets<render::MaskMaterial>>,
    mut images: ResMut<Assets<Image>>,
    formats: Option<Res<bevy::image::CompressedImageFormatSupport>>,
    mut batches: Query<(Entity, &mut Transform, &mut Visibility), (With<render::UiCursorBatch>, Without<UiCamera>)>,
) {
    let Some(rt) = rt else { return };
    let Ok(window) = windows.single() else { return };
    let pointer = match virt.pos {
        Some(p) => Some(p),
        None => window.cursor_position().map(|p| p * window.resolution.scale_factor()),
    };
    let mut g = lua::ui(&rt.lua);
    // nothing held and nothing drawn: the common case costs a comparison
    if g.drag.held.is_none() && batches.is_empty() {
        state.cursor_key = None;
        return;
    }
    render::sync_cursor(&mut commands, &mut state, &mut g, pointer, &mut meshes, &mut materials, &mut mask_materials, &mut images, formats.as_deref(), &mut batches);
}

/// Physical key edges -> generic capture or edit focus, preserving real release timing.
fn ui_keyboard(
    rt: Option<NonSend<UiRuntime>>,
    mut keys: MessageReader<KeyboardInput>,
    mut buttons: MessageReader<MouseButtonInput>,
    mut lost: MessageReader<KeyboardFocusLost>,
    mut ime: MessageReader<bevy::window::Ime>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut stats: ResMut<UiStats>,
) {
    let Some(rt) = rt else {
        keys.clear();
        buttons.clear();
        lost.clear();
        ime.clear();
        stats.keyboard_focus = false;
        return;
    };
    lua::keyboard::begin_frame(&rt.lua);
    let focus_lost = lost.read().next().is_some();
    lost.clear();
    let window = windows.single().ok();
    let events: Vec<_> = ime.read().filter(|ev| {
        let id = match ev {
            bevy::window::Ime::Enabled { window }
            | bevy::window::Ime::Disabled { window }
            | bevy::window::Ime::Preedit { window, .. }
            | bevy::window::Ime::Commit { window, .. } => *window,
        };
        Some(id) == window
    }).collect();
    for (i, event) in events.iter().enumerate() {
        // Winit emits an empty preedit immediately before a result. Native handles
        // GCS_COMPSTR + GCS_RESULTSTR together and publishes only the resulting text.
        if matches!(event, bevy::window::Ime::Preedit { value, .. } if value.is_empty())
            && events.get(i + 1).is_some_and(|next| matches!(next, bevy::window::Ime::Commit { value, .. } if !value.is_empty()))
        { continue; }
        rt.ime_event(event);
    }
    for ev in keys.read() {
        rt.key_event(ev.key_code, ev.state == bevy::input::ButtonState::Pressed, ev.text.as_deref());
    }
    for ev in buttons.read() {
        if let Some(name) = lua::keyboard::button_name(ev.button) {
            rt.button_event(name, ev.state == bevy::input::ButtonState::Pressed);
        }
    }
    if focus_lost {
        lua::keyboard::focus_lost(&rt.lua);
    }
    stats.keyboard_focus = rt.keyboard_captured();
}

fn ui_ime_window(rt: Option<NonSend<UiRuntime>>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let Some(rt) = rt else { return };
    let Ok(mut window) = windows.single_mut() else { return };
    let (enabled, point) = lua::edit::ime::window(&rt.lua, window.focused);
    if window.ime_enabled != enabled { window.ime_enabled = enabled; }
    if let Some(point) = point {
        let s = rt.ui_scale() / window.resolution.scale_factor();
        let point = Vec2::new(point[0] * s, point[1] * s);
        if window.ime_position != point { window.ime_position = point; }
    }
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
    mut mask_materials: ResMut<Assets<render::MaskMaterial>>,
    mut images: ResMut<Assets<Image>>,
    formats: Option<Res<bevy::image::CompressedImageFormatSupport>>,
    batches: Query<Entity, With<UiBatch>>,
    mut camera: Query<(&mut Transform, &mut Projection), With<UiCamera>>,
    mut virt: ResMut<UiVirtualPointer>,
) {
    let Some(mut rt) = rt else { return };
    let Ok(window) = windows.single() else { return };
    let t0 = std::time::Instant::now();
    let p_frame = prof::scope("frame");
    if prof::enabled() && time.delta_secs() > 0.035 {
        info!("aa-ui: slow frame {:.0} ms", time.delta_secs() * 1000.0);
    }
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
    let wheel = scroll.delta.y + std::mem::take(&mut virt.wheel);
    let (vpress, vrelease) = (
        std::mem::take(&mut virt.press),
        std::mem::take(&mut virt.release),
    );
    let p_input = prof::scope("input");
    input::update_buttons(
        lua,
        &mut rt.mouse,
        cursor,
        input::MouseButtons {
            press: buttons.just_pressed(MouseButton::Left) || vpress,
            release: buttons.just_released(MouseButton::Left) || vrelease,
            right_press: buttons.just_pressed(MouseButton::Right)
                || std::mem::take(&mut virt.right_press),
            right_release: buttons.just_released(MouseButton::Right)
                || std::mem::take(&mut virt.right_release),
        },
        wheel,
    );
    drop(p_input);
    // Script time.
    lua::tick(lua, time.delta_secs() * 1000.0);
    // Rebuild the meshes when anything changed.
    let pressed = rt.mouse.pressed;
    let mut g = lua::ui(lua);
    let u = &mut *g;
    let content = u.tree.settle();
    if content != state.generation || u.fonts.atlas_dirty || state.screen != (pw as u32, ph as u32)
    {
        prof::count("rebuild");
        if content == state.generation {
            prof::change(|| if u.fonts.atlas_dirty { "glyph atlas".into() } else { "window size".into() });
        }
        let p_build = prof::scope("build_quads");
        let (quads, background_quads) = render::build_quads_split(u, pressed);
        drop(p_build);
        let _p_sync = prof::scope("sync_meshes");
        debug!(
            "aa-ui: rebuild {} quads, {} background (split layer {:?})",
            quads.len(),
            background_quads,
            state.background_layer
        );
        render::sync_atlas(u, &mut state, &mut images);
        render::sync_batches(
            &mut commands,
            &mut state,
            &quads,
            background_quads,
            &mut meshes,
            &mut materials,
            &mut mask_materials,
            &mut images,
            formats.as_deref(),
            &batches,
        );
        state.generation = content;
        state.screen = (pw as u32, ph as u32);
        stats.quads = state.quads;
        stats.draw_calls = state.draw_calls;
        stats.rebuilds += 1;
        stats.mesh_uploads = state.uploads;
    }
    stats.keyboard_focus = u.keyboard.captured(&u.tree, u.focus);
    drop(g);
    stats.frame_us = t0.elapsed().as_secs_f32() * 1e6;
    drop(p_frame);
    prof::frame_end();
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
