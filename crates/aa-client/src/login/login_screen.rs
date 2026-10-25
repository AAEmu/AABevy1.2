//! The login screen (`Screen::Login`): account, password and server typed in, Connect, then World Select.
//!
//! Plain Bevy UI on its own camera, no Lua UI. Bevy 0.20's `EditableText` has no password masking, so the three
//! fields are a small widget of our own ([`Field`]): click or Tab to focus, typing, Backspace / Delete, arrows,
//! Home / End, keyboard selection, copy/cut/paste, a blinking caret, masked passwords, Enter to connect, Esc to cancel.
//! Selected text is drawn in separate highlighted runs on either side of the caret.
//!
//! * Art: [`art`] animates `assets/login/manifest.json` (parallax, drifting mist, motes, breathing). The folder is
//!   read through the asset source `login` ([`register_assets`], called by `main` before the asset plugin) so it
//!   does not matter where the exe runs from. Without the files the screen is a dark teal gradient and a plain panel.
//! * Flow: Connect updates `Credentials`, sends `Command::Login` and waits for `LoginSucceeded` / `WorldList`
//!   (fade out, then `Screen::WorldSelect`), `LoginFailed` / `ConnectFailed` (the message is shown, retry), or
//!   [`CONNECT_TIMEOUT`]. The password only lives in the form and in `Credentials`; it is never logged or saved.
//! * Entered again when World Select's Exit Game, a lost connection, or the HUD's disconnect dialog leave the session
//!   ([`back_to_login`], `Notice`).
//! * Dev runs skip the screen ([`enabled`]): any `--flag` besides `--size`, `--mute`, `--sound` means "as before", so
//!   `--account`, `--goto`, `--shot`, ... log in at startup. `--login-screen` shows it anyway (prefilled from
//!   `--account` / `--login`), `--login-type USER PASS HOST T` types and presses Connect at T seconds,
//!   `--login-no-connect` types without pressing it.
//! * Remembers the user and server (`login.toml`, see [`config`]), never the password.

mod art;
mod config;
mod field;
#[allow(dead_code)]
mod fixture;
#[cfg(test)]
mod tests;

use field::Field;

#[allow(dead_code)]
pub(crate) fn selection_fixture() {
    fixture::run();
}

use std::sync::OnceLock;

use aa_audio::{Category, Engine, PlayOptions};
use aa_net::state::{Status as NetStatus, Worlds};
use aa_net::{Command, Event, NetCommand, NetEvent, Phase, Secret};
use bevy::app::AppExit;
use bevy::asset::io::AssetSourceBuilder;
use bevy::camera::ClearColorConfig;
use bevy::clipboard::Clipboard;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::text::{Font, FontSize, FontSource, TextColor, TextFont, TextLayout};
use bevy::ui::{ComputedNode, UiGlobalTransform, UiTargetCamera};
use bevy::window::PrimaryWindow;

use super::{NativeLoginStarted, UiBuilt, host};
use crate::Screen;
use crate::ingame::enter::{Credentials, flag};
use crate::sound::Packs;
use art::{Art, Canvas, PanelDef};

/// Seconds without an answer before the screen gives up (the session's own request timeout is 20 s, but a
/// connect to a black-holed address is not covered by it).
const CONNECT_TIMEOUT: f32 = 25.0;
const FADE_IN: f32 = 0.9;
const FADE_OUT: f32 = 0.5;

/// Whether the login screen is part of this run (see the module docs).
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|a| a == "--enter-world" || a == "--script") {
            return false;
        }
        if args.iter().any(|a| a == "--login-screen" || a == "--login-type") {
            return true;
        }
        !args.iter().any(|a| a.starts_with("--") && !matches!(a.as_str(), "--size" | "--mute" | "--sound"))
    })
}

/// `run_if` of the startup login: the screens log in on their own when this one is not shown.
pub fn skipped() -> bool {
    !enabled()
}

/// The first screen.
pub fn initial_screen() -> Screen {
    if enabled() { Screen::Login } else { Screen::WorldSelect }
}

/// Registers the asset source of the art folder. Call before the asset plugin is added.
pub fn register_assets(app: &mut App) {
    if !enabled() {
        return;
    }
    register_art_source(app);
}

fn register_art_source(app: &mut App) {
    match art::assets_dir() {
        Some(dir) => {
            app.register_asset_source(art::SOURCE, AssetSourceBuilder::platform_default(&dir.to_string_lossy(), None));
        }
        None => warn!("login screen: no assets/login folder, using the plain background"),
    }
}

/// A line the login screen shows when it comes up (why the session ended).
#[derive(Resource, Default)]
pub struct Notice(pub Option<String>);

/// Leave the session for the login screen, telling the player why.
pub fn back_to_login(message: &str, notice: &mut Notice, next: &mut NextState<Screen>) {
    let text = match message {
        "" | "disconnected" | "logged out" => "You were disconnected from the server.".to_string(),
        m => format!("Disconnected: {m}"),
    };
    notice.0 = Some(text);
    next.set(Screen::Login);
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Notice>();
    if !enabled() {
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    let mut saved = config::load();
    if let Some(user) = flag(&args, "--account").or_else(|| flag(&args, "--user")) {
        saved.user = user;
    }
    if let Some(server) = flag(&args, "--login") {
        saved.server = server;
    }
    let script = args.iter().position(|a| a == "--login-type").and_then(|i| {
        let v = args.get(i + 1..i + 5)?;
        Some(Script { user: v[0].clone(), password: v[1].clone(), server: v[2].clone(), at: v[3].parse().ok()?, connect: !args.iter().any(|a| a == "--login-no-connect"), done: false })
    });
    if let Some(manifest) = art::assets_dir().and_then(|d| art::Manifest::load(&d)) {
        app.insert_resource(Art::new(manifest));
    }
    app.add_message::<Act>()
        .insert_resource(LoginForm::new(saved, script))
        .init_resource::<FormBuilt>()
        .add_systems(OnEnter(Screen::Login), (prepare, spawn_screen).chain())
        .add_systems(OnExit(Screen::Login), leave)
        .add_systems(Update, art::mipmaps)
        .add_systems(
            Update,
            (
                (rebuild, art::animate.run_if(resource_exists::<Art>), keyboard, mouse, script_input, act, net_events, drive, sync, scroll_fields).chain(),
            )
                .run_if(in_state(Screen::Login)),
        );
}

// ---- the form ---------------------------------------------------------------------------------

const USER: usize = 0;
const PASSWORD: usize = 1;
const SERVER: usize = 2;
const LABELS: [&str; 3] = ["Account", "Password", "Server"];
const MAX_LEN: [usize; 3] = [32, 64, 80];

#[derive(Clone, PartialEq, Debug)]
enum Status {
    Idle,
    Connecting { since: f32 },
    Error(String),
    /// Logged in: fading out, `at` is when it started.
    Done { at: f32 },
}

/// `--login-type`.
struct Script {
    user: String,
    password: String,
    server: String,
    at: f32,
    connect: bool,
    done: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hover {
    None,
    Connect,
    Exit,
}

#[derive(Resource)]
struct LoginForm {
    fields: [Field; 3],
    focus: usize,
    status: Status,
    /// Seconds since the screen came up, and since the last edit (caret blink).
    age: f32,
    blink: f32,
    hover: Hover,
    /// The address of the attempt in flight, for the messages.
    addr: String,
    saved: config::Saved,
    script: Option<Script>,
}

impl LoginForm {
    fn new(saved: config::Saved, script: Option<Script>) -> LoginForm {
        LoginForm {
            fields: [Field::with(&saved.user), Field::default(), Field::with(&saved.server)],
            focus: USER,
            status: Status::Idle,
            age: 0.0,
            blink: 0.0,
            hover: Hover::None,
            addr: String::new(),
            saved,
            script,
        }
    }

    fn busy(&self) -> bool {
        matches!(self.status, Status::Connecting { .. } | Status::Done { .. })
    }
}

#[derive(Message, Clone, Copy, Debug)]
enum Act {
    Submit,
    Cancel,
    Exit,
}

// ---- entities -----------------------------------------------------------------------------------

#[derive(Component)]
struct LoginEntity;
#[derive(Component)]
struct Holder;
#[derive(Component)]
struct Fade;
#[derive(Component)]
struct FieldBox(usize);
#[derive(Component)]
struct FieldRow(usize);
#[derive(Component)]
struct FieldText(usize, usize);
#[derive(Component)]
struct Caret(usize);
#[derive(Component)]
struct StatusLine;
#[derive(Component)]
struct ConnectButton;
#[derive(Component)]
struct ConnectLabel;
#[derive(Component)]
struct ExitButton;

/// The canvas the form was built for (rebuilt when the window changes).
#[derive(Resource, Default)]
struct FormBuilt(Option<Canvas>);

#[derive(Resource)]
struct Fonts {
    /// The title, labels and buttons.
    heading: Handle<Font>,
    /// What is typed and the messages.
    text: Handle<Font>,
}

const GOLD: Color = Color::srgb(0.86, 0.72, 0.42);
const GOLD_DIM: Color = Color::srgba(0.62, 0.50, 0.28, 0.9);
const INK: Color = Color::srgb(0.93, 0.91, 0.84);
const MUTED: Color = Color::srgb(0.58, 0.72, 0.71);
const ERROR: Color = Color::srgb(0.96, 0.50, 0.44);
const FIELD_BG: Color = Color::srgba(0.02, 0.06, 0.08, 0.78);
const TEAL: Color = Color::srgb(0.03, 0.09, 0.11);

fn load_font(fonts: &mut Assets<Font>, path: &str) -> Handle<Font> {
    match aa_pak::Pak::global().read(path) {
        Ok(bytes) => fonts.add(Font::from_bytes(bytes)),
        Err(e) => {
            warn!("login screen: font {path}: {e}");
            Handle::default()
        }
    }
}

/// First half of entering: the previous screen's Lua UI goes, the fonts and the form are made ready.
fn prepare(world: &mut World) {
    // the previous screen's Lua UI would otherwise come back with the old stage
    if world.resource::<UiBuilt>().0.is_some() {
        host::clear(world);
        world.resource_mut::<UiBuilt>().0 = None;
    }
    if world.get_resource::<Fonts>().is_none() {
        let mut fonts = world.resource_mut::<Assets<Font>>();
        let fonts = Fonts {
            heading: load_font(&mut fonts, "ui/font/librebaskerville-bold.ttf"),
            text: load_font(&mut fonts, "ui/font/flareserif_821_roman.ttf"),
        };
        world.insert_resource(fonts);
    }
    let notice = world.resource_mut::<Notice>().0.take();
    let mut form = world.resource_mut::<LoginForm>();
    form.fields[PASSWORD].set("");
    form.status = notice.map_or(Status::Idle, Status::Error);
    form.age = 0.0;
    form.blink = 0.0;
    form.focus = if form.fields[USER].text.is_empty() { USER } else { PASSWORD };
    world.resource_mut::<FormBuilt>().0 = None;
}

/// Second half: the camera, the layers and the (still empty) form holder.
fn spawn_screen(mut commands: Commands, art: Option<Res<Art>>, assets: Res<AssetServer>, mut net: MessageWriter<NetCommand>) {
    // nothing of an old session stays up behind the screen
    net.write(NetCommand(Command::Disconnect));
    let camera = commands.spawn((Name::new("login camera"), LoginEntity, Camera2d, Camera { order: 30, clear_color: ClearColorConfig::Custom(TEAL), ..default() })).id();
    let full = || Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() };
    let root = commands
        .spawn((
            Name::new("login root"),
            LoginEntity,
            UiTargetCamera(camera),
            Node { overflow: Overflow::clip(), ..full() },
            // the fallback below the art: a dark teal gradient
            BackgroundGradient::from(LinearGradient::new(
                LinearGradient::TO_BOTTOM,
                vec![ColorStop::percent(Color::srgb(0.02, 0.07, 0.09), 0.0), ColorStop::percent(Color::srgb(0.05, 0.14, 0.16), 100.0)],
            )),
        ))
        .id();
    // the art moves and breathes by fractions of a pixel: rounded to whole pixels it jumps, and every
    // change of the rounded size samples the whole picture anew (the line work flickers)
    let layers = commands.spawn((Name::new("login art"), full(), LayoutConfig { use_rounding: false }, ChildOf(root))).id();
    if let Some(art) = &art {
        art::spawn(&mut commands, layers, &assets, art);
    }
    commands.spawn((Name::new("login form"), Holder, full(), ChildOf(root)));
    commands.spawn((Name::new("login fade"), Fade, full(), BackgroundColor(Color::BLACK), ChildOf(root)));
}

fn leave(mut commands: Commands, entities: Query<Entity, With<LoginEntity>>) {
    for e in &entities {
        commands.entity(e).despawn();
    }
}

fn window_size(window: &Query<&Window, With<PrimaryWindow>>) -> Option<Vec2> {
    let w = window.single().ok()?;
    let size = Vec2::new(w.resolution.physical_width() as f32, w.resolution.physical_height() as f32);
    (size.x >= 1.0 && size.y >= 1.0).then_some(size)
}

fn text_bundle(font: &Handle<Font>, text: &str, size: f32, color: Color) -> impl Bundle {
    (Text::new(text), TextFont { font: FontSource::Handle(font.clone()), font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

/// Builds the form for the current window size, again when it changes.
#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    window: Query<&Window, With<PrimaryWindow>>,
    mut built: ResMut<FormBuilt>,
    holder: Query<Entity, With<Holder>>,
    fonts: Res<Fonts>,
    art: Option<Res<Art>>,
    assets: Res<AssetServer>,
) {
    let (Some(size), Ok(holder)) = (window_size(&window), holder.single()) else { return };
    let canvas = Canvas::fit(size);
    if built.0 == Some(canvas) {
        return;
    }
    built.0 = Some(canvas);
    let plain = PanelDef::plain();
    let panel = art.as_ref().map_or(&plain, |a| &a.manifest.panel);
    let s = canvas.s;
    let (heading, text) = (&fonts.heading, &fonts.text);

    let top_left = canvas.to_window(panel.centre - panel.size / 2.0);
    let panel_size = panel.size * s;
    let inset = panel.inset;
    let content_pos = top_left + Vec2::new(inset[0], inset[1]) * s;
    let content_size = panel_size - Vec2::new(inset[0] + inset[2], inset[1] + inset[3]) * s;

    commands.entity(holder).despawn_children();
    commands.entity(holder).with_children(|h| {
        // the panel: the art, or a plain colour with a gold edge
        let art_panel = art.as_ref().filter(|_| !panel.file.is_empty());
        let mut p = h.spawn(Node {
            position_type: PositionType::Absolute,
            left: px(top_left.x),
            top: px(top_left.y),
            width: px(panel_size.x),
            height: px(panel_size.y),
            border: UiRect::all(px(if art_panel.is_some() { 0.0 } else { 2.0 * s })),
            border_radius: BorderRadius::all(px(8.0 * s)),
            ..default()
        });
        match art_panel {
            Some(a) => p.insert(ImageNode::new(art::panel_image(&assets, a))),
            None => p.insert((BackgroundColor(Color::srgba(0.04, 0.10, 0.12, 0.92)), BorderColor::all(GOLD_DIM))),
        };

        h.spawn(Node {
            position_type: PositionType::Absolute,
            left: px(content_pos.x),
            top: px(content_pos.y),
            width: px(content_size.x),
            height: px(content_size.y),
            flex_direction: FlexDirection::Column,
            row_gap: px(7.0 * s),
            ..default()
        })
        .with_children(|c| {
            // title
            c.spawn(Node { width: percent(100), justify_content: JustifyContent::Center, margin: UiRect::bottom(px(2.0 * s)), ..default() })
                .with_children(|t| {
                    t.spawn(text_bundle(heading, "AAEmu", 38.0 * s, GOLD));
                });
            for (i, label) in LABELS.iter().enumerate() {
                c.spawn(text_bundle(heading, label, 17.0 * s, MUTED));
                c.spawn((
                    FieldBox(i),
                    Node {
                        width: percent(100),
                        height: px(42.0 * s),
                        flex_shrink: 0.0,
                        border: UiRect::all(px(2.0 * s)),
                        border_radius: BorderRadius::all(px(5.0 * s)),
                        padding: UiRect::horizontal(px(12.0 * s)),
                        align_items: AlignItems::Center,
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BackgroundColor(FIELD_BG),
                    BorderColor::all(GOLD_DIM),
                ))
                .with_children(|f| {
                    f.spawn((FieldRow(i), Node { align_items: AlignItems::Center, flex_shrink: 0.0, position_type: PositionType::Relative, ..default() })).with_children(|r| {
                        let font = |color| (TextFont { font: FontSource::Handle(text.clone()), font_size: FontSize::Px(24.0 * s), ..default() }, TextColor(color), TextLayout::no_wrap());
                        for part in 0..2 {
                            r.spawn((FieldText(i, part), Text::new(""), font(INK), BackgroundColor(Color::NONE), Node { flex_shrink: 0.0, ..default() }));
                        }
                        r.spawn((Caret(i), Node { width: px(2.0 * s), height: px(26.0 * s), flex_shrink: 0.0, margin: UiRect::horizontal(px(1.0 * s)), ..default() }, BackgroundColor(GOLD)));
                        for part in 2..4 {
                            r.spawn((FieldText(i, part), Text::new(""), font(INK), BackgroundColor(Color::NONE), Node { flex_shrink: 0.0, ..default() }));
                        }
                    });
                });
            }
            // status / error line
            c.spawn(Node { width: percent(100), height: px(40.0 * s), flex_shrink: 0.0, ..default() }).with_children(|l| {
                l.spawn((StatusLine, Text::new(""), TextFont { font: FontSource::Handle(text.clone()), font_size: FontSize::Px(18.0 * s), ..default() }, TextColor(ERROR), Node { width: percent(100), ..default() }));
            });
            // Connect
            c.spawn((
                ConnectButton,
                Node {
                    width: percent(100),
                    height: px(48.0 * s),
                    flex_shrink: 0.0,
                    border: UiRect::all(px(2.0 * s)),
                    border_radius: BorderRadius::all(px(6.0 * s)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.10, 0.22, 0.24, 0.9)),
                BorderColor::all(GOLD),
            ))
            .with_children(|b| {
                b.spawn((ConnectLabel, text_bundle(heading, "Connect", 26.0 * s, GOLD)));
            });
        });

        // the hint under the panel and the Exit button
        h.spawn(Node { position_type: PositionType::Absolute, left: px(top_left.x), top: px(top_left.y + panel_size.y + 10.0 * s), width: px(panel_size.x), justify_content: JustifyContent::Center, ..default() })
            .with_children(|n| {
                n.spawn(text_bundle(text, "Tab: next field    Enter: connect", 19.0 * s, Color::srgba(0.70, 0.82, 0.80, 0.9)));
            });
        h.spawn((
            ExitButton,
            Node { position_type: PositionType::Absolute, right: px(44.0 * s), bottom: px(36.0 * s), padding: UiRect::axes(px(26.0 * s), px(8.0 * s)), border: UiRect::all(px(2.0 * s)), border_radius: BorderRadius::all(px(6.0 * s)), ..default() },
            BackgroundColor(Color::srgba(0.02, 0.06, 0.08, 0.6)),
            BorderColor::all(GOLD_DIM),
        ))
        .with_children(|b| {
            b.spawn(text_bundle(heading, "Exit", 22.0 * s, MUTED));
        });
    });
}

// ---- input ---------------------------------------------------------------------------------------

fn keyboard(
    mut events: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut form: ResMut<LoginForm>,
    clipboard: Option<ResMut<Clipboard>>,
    mut acts: MessageWriter<Act>,
) {
    let mut clipboard = clipboard;
    let events: Vec<_> = events.read().collect();
    let mut modifiers = [KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::ShiftLeft, KeyCode::ShiftRight]
        .map(|code| keys.pressed(code));
    // InputPlugin has already applied the entire batch. Recover its starting
    // modifier state, then replay transitions so releases after a shortcut do
    // not erase that shortcut's Ctrl/Shift. Keep the two physical sides distinct.
    for event in events.iter().rev() {
        if !event.repeat && let Some(index) = modifier_index(event.key_code) {
            modifiers[index] = event.state == ButtonState::Released;
        }
    }
    for ev in events {
        if let Some(index) = modifier_index(ev.key_code) {
            modifiers[index] = ev.state == ButtonState::Pressed;
            continue;
        }
        if ev.state != ButtonState::Pressed {
            continue;
        }
        if form.age < 0.2 && !matches!(ev.logical_key, Key::Escape) {
            continue;
        }
        let editing = !form.busy();
        let focus = form.focus;
        let ctrl = modifiers[0] || modifiers[1];
        let shift = modifiers[2] || modifiers[3];
        form.blink = 0.0;
        match &ev.logical_key {
            Key::Enter => {
                if !ev.repeat {
                    acts.write(Act::Submit);
                }
            }
            Key::Escape => {
                if form.busy() {
                    acts.write(Act::Cancel);
                }
            }
            Key::Tab => {
                if !ev.repeat && editing {
                    form.focus = if shift { (focus + 2) % 3 } else { (focus + 1) % 3 };
                }
            }
            Key::ArrowDown if editing => form.focus = (focus + 1) % 3,
            Key::ArrowUp if editing => form.focus = (focus + 2) % 3,
            Key::Backspace if editing => {
                form.fields[focus].backspace();
                clear_error(&mut form);
            }
            Key::Delete if editing => {
                form.fields[focus].delete();
                clear_error(&mut form);
            }
            Key::ArrowLeft if editing => form.fields[focus].left(shift),
            Key::ArrowRight if editing => form.fields[focus].right(shift),
            Key::Home if editing => form.fields[focus].home(shift),
            Key::End if editing => form.fields[focus].end(shift),
            Key::Character(c) if ctrl && editing && c.eq_ignore_ascii_case("a") => form.fields[focus].select_all(),
            Key::Character(c) if ctrl && editing && (c.eq_ignore_ascii_case("c") || c.eq_ignore_ascii_case("x")) => {
                // Masked fields support selection/replacement without exporting the secret.
                if focus != PASSWORD
                    && !form.fields[focus].selected().is_empty()
                    && copy_text(clipboard.as_deref_mut(), form.fields[focus].selected())
                    && c.eq_ignore_ascii_case("x")
                {
                    form.fields[focus].delete();
                    clear_error(&mut form);
                }
            }
            Key::Character(c) if ctrl && editing && c.eq_ignore_ascii_case("v") => {
                if let Some(text) = pasted_text(clipboard.as_deref_mut()) {
                    form.fields[focus].insert(&text, MAX_LEN[focus]);
                    clear_error(&mut form);
                }
            }
            _ => {
                if !ctrl
                    && editing
                    && let Some(text) = &ev.text
                {
                    form.fields[focus].insert(text, MAX_LEN[focus]);
                    clear_error(&mut form);
                }
            }
        }
    }
}

fn modifier_index(code: KeyCode) -> Option<usize> {
    match code {
        KeyCode::ControlLeft => Some(0),
        KeyCode::ControlRight => Some(1),
        KeyCode::ShiftLeft => Some(2),
        KeyCode::ShiftRight => Some(3),
        _ => None,
    }
}

// Bevy's current clipboard resource was built without system_clipboard. Use the
// same Windows Unicode backend as aa-ui instead of its process-local buffer.
fn copy_text(clipboard: Option<&mut Clipboard>, text: &str) -> bool {
    #[cfg(windows)]
    {
        let _ = clipboard;
        clipboard_win::set_clipboard_string(text).is_ok()
    }
    #[cfg(not(windows))]
    {
        clipboard.is_some_and(|clipboard| clipboard.set_text(text).is_ok())
    }
}

fn pasted_text(clipboard: Option<&mut Clipboard>) -> Option<String> {
    #[cfg(windows)]
    {
        let _ = clipboard;
        clipboard_win::get_clipboard_string().ok()
    }
    #[cfg(not(windows))]
    {
        clipboard?.fetch_text().poll_result()?.ok()
    }
}

fn clear_error(form: &mut LoginForm) {
    if matches!(form.status, Status::Error(_)) {
        form.status = Status::Idle;
    }
}

fn hit(window: &Window, node: &ComputedNode, transform: &UiGlobalTransform) -> bool {
    window.cursor_position().is_some_and(|p| node.contains_point(*transform, p * window.resolution.scale_factor()))
}

#[allow(clippy::type_complexity)]
fn mouse(
    window: Query<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    fields: Query<(&FieldBox, &ComputedNode, &UiGlobalTransform)>,
    connect: Query<(&ComputedNode, &UiGlobalTransform), With<ConnectButton>>,
    exit: Query<(&ComputedNode, &UiGlobalTransform), With<ExitButton>>,
    mut form: ResMut<LoginForm>,
    mut acts: MessageWriter<Act>,
    sound: (Option<Res<Engine>>, Option<Res<Packs>>),
) {
    let Ok(window) = window.single() else { return };
    let over_connect = connect.iter().any(|(n, t)| hit(window, n, t));
    let over_exit = exit.iter().any(|(n, t)| hit(window, n, t));
    let hover = if over_connect { Hover::Connect } else if over_exit { Hover::Exit } else { Hover::None };
    if hover != form.hover {
        if hover != Hover::None {
            play(&sound.0, &sound.1, "default_over");
        }
        form.hover = hover;
    }
    if !buttons.just_pressed(MouseButton::Left) || form.age < 0.2 {
        return;
    }
    if over_exit {
        acts.write(Act::Exit);
    } else if over_connect {
        acts.write(Act::Submit);
    } else if !form.busy()
        && let Some((FieldBox(i), ..)) = fields.iter().find(|(_, n, t)| hit(window, n, t))
    {
        form.focus = *i;
        form.blink = 0.0;
    }
}

/// A UI sound of the `button` pack (`default_left_up` is the click).
fn play(engine: &Option<Res<Engine>>, packs: &Option<Res<Packs>>, item: &str) {
    let (Some(engine), Some(path)) = (engine, packs.as_ref().and_then(|p| p.get()).and_then(|p| p.path("button", item))) else { return };
    engine.play_with(path, None, PlayOptions { category: Some(Category::Ui), ..default() });
}

/// `--login-type USER PASS HOST T`.
fn script_input(time: Res<Time>, mut form: ResMut<LoginForm>, mut acts: MessageWriter<Act>) {
    let t = time.elapsed_secs();
    let Some(script) = &mut form.script else { return };
    if script.done || t < script.at {
        return;
    }
    script.done = true;
    let (user, password, server, connect) = (script.user.clone(), script.password.clone(), script.server.clone(), script.connect);
    form.fields[USER].set(&user);
    form.fields[PASSWORD].set(&password);
    form.fields[SERVER].set(&server);
    form.focus = PASSWORD;
    info!("login screen: --login-type filled the form as {user} for {server}");
    if connect {
        acts.write(Act::Submit);
    }
}

// ---- actions -------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn act(
    mut acts: MessageReader<Act>,
    mut form: ResMut<LoginForm>,
    mut creds: ResMut<Credentials>,
    mut started: ResMut<NativeLoginStarted>,
    mut net: MessageWriter<NetCommand>,
    mut exit: MessageWriter<AppExit>,
    time: Res<Time>,
    engine: Option<Res<Engine>>,
    packs: Option<Res<Packs>>,
) {
    for act in acts.read().copied().collect::<Vec<_>>() {
        match act {
            Act::Exit => {
                play(&engine, &packs, "default_left_up");
                exit.write(AppExit::Success);
            }
            Act::Cancel => {
                net.write(NetCommand(Command::Disconnect));
                form.status = Status::Idle;
            }
            Act::Submit => {
                if form.busy() {
                    continue;
                }
                play(&engine, &packs, "default_left_up");
                let user = form.fields[USER].text.trim().to_string();
                let password = form.fields[PASSWORD].text.clone();
                let problem = if user.is_empty() {
                    Some((USER, "Enter your account name.".to_string()))
                } else if password.is_empty() {
                    Some((PASSWORD, "Enter your password.".to_string()))
                } else {
                    config::parse_address(&form.fields[SERVER].text).err().map(|e| (SERVER, e))
                };
                if let Some((field, message)) = problem {
                    form.focus = field;
                    form.status = Status::Error(message);
                    continue;
                }
                let addr = config::parse_address(&form.fields[SERVER].text).unwrap_or_default();
                info!("login screen: logging in as {user} at {addr}");
                creds.user = user;
                creds.login = addr.clone();
                creds.password = Secret::new(password);
                started.0 = true;
                net.write(NetCommand(creds.command()));
                form.addr = addr;
                form.status = Status::Connecting { since: time.elapsed_secs() };
            }
        }
    }
}

/// A server message in the words of the screen.
fn friendly(message: &str, addr: &str) -> String {
    let lower = message.to_lowercase();
    if lower.contains("cannot reach") {
        format!("Cannot reach the login server {addr}. Check the address and that the server is running.")
    } else if lower.contains("timed out") {
        format!("The login server {addr} did not answer in time (connection timeout).")
    } else if let Some((_, reason)) = lower.split_once("login denied (reason ").and_then(|(_, r)| r.split_once("): ")) {
        let reason = reason.trim();
        if reason.contains("wrong user name or password") {
            "Login denied: wrong account name or password.".to_string()
        } else {
            let mut c = reason.chars();
            format!("Login denied: {}.", c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default())
        }
    } else if lower.contains("login server") || lower.contains("disconnected") {
        format!("The connection to {addr} was lost.")
    } else {
        let mut c = message.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    }
}

fn net_events(mut events: MessageReader<NetEvent>, mut form: ResMut<LoginForm>, time: Res<Time>) {
    for NetEvent(event) in events.read() {
        if !matches!(form.status, Status::Connecting { .. }) {
            continue;
        }
        match event {
            Event::LoginSucceeded(_) | Event::WorldList(_) => {
                info!("login screen: logged in");
                form.status = Status::Done { at: time.elapsed_secs() };
            }
            Event::LoginFailed { message } | Event::ConnectFailed { message } | Event::Disconnected { message } => {
                warn!("login screen: {message}");
                form.status = Status::Error(friendly(message, &form.addr.clone()));
            }
            _ => {}
        }
    }
}

/// The clock of the screen: fade in, the timeout, the fade out and the hand-over to World Select.
fn drive(
    time: Res<Time>,
    mut form: ResMut<LoginForm>,
    mut net: MessageWriter<NetCommand>,
    mut next: ResMut<NextState<Screen>>,
    status: Res<NetStatus>,
    worlds: Res<Worlds>,
) {
    let dt = time.delta_secs();
    form.age += dt;
    form.blink += dt;
    let now = time.elapsed_secs();
    match form.status.clone() {
        Status::Connecting { since } if now - since > CONNECT_TIMEOUT => {
            net.write(NetCommand(Command::Disconnect));
            form.status = Status::Error(format!("The login server {} did not answer in time (connection timeout).", form.addr));
        }
        Status::Done { at } => {
            let ready = status.phase == Phase::WorldSelect && worlds.account.is_some();
            if (now - at > FADE_OUT && ready) || now - at > 10.0 {
                if form.script.is_none() {
                    let saved = config::Saved { user: form.fields[USER].text.trim().to_string(), server: form.addr.clone() };
                    if saved != form.saved {
                        config::save(&saved);
                        form.saved = saved;
                    }
                }
                next.set(Screen::WorldSelect);
            }
        }
        _ => {}
    }
}

// ---- drawing the form ----------------------------------------------------------------------------

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn sync(
    time: Res<Time>,
    form: Res<LoginForm>,
    mut texts: Query<(&mut Text, &mut TextColor, Option<&FieldText>, Option<&StatusLine>, Option<&ConnectLabel>)>,
    mut carets: Query<(&Caret, &mut BackgroundColor), Without<FieldBox>>,
    mut selections: Query<(&FieldText, &mut BackgroundColor), (Without<FieldBox>, Without<Caret>)>,
    mut boxes: Query<(&FieldBox, &mut BorderColor, &mut BackgroundColor), Without<Caret>>,
    mut buttons: Query<(Option<&ConnectButton>, Option<&ExitButton>, &mut BorderColor, &mut BackgroundColor), (Or<(With<ConnectButton>, With<ExitButton>)>, Without<FieldBox>, Without<Caret>, Without<FieldText>)>,
    mut fade: Query<&mut BackgroundColor, (With<Fade>, Without<FieldBox>, Without<Caret>, Without<ConnectButton>, Without<ExitButton>, Without<FieldText>)>,
) {
    let busy = form.busy();
    let shown: [[String; 4]; 3] = std::array::from_fn(|i| form.fields[i].shown(i == PASSWORD));
    for (mut text, mut color, field, status, label) in &mut texts {
        let want = if let Some(FieldText(i, part)) = field {
            Some(shown[*i][*part].clone())
        } else if status.is_some() {
            let dots = ".".repeat((time.elapsed_secs() * 2.5) as usize % 4);
            let (line, c) = match &form.status {
                Status::Error(m) => (m.clone(), ERROR),
                Status::Connecting { .. } => (format!("Connecting to {}{dots}", form.addr), MUTED),
                Status::Done { .. } => ("Logged in.".to_string(), MUTED),
                Status::Idle => (String::new(), MUTED),
            };
            color.0 = c;
            Some(line)
        } else if label.is_some() {
            Some(if busy { "Connecting..." } else { "Connect" }.to_string())
        } else {
            None
        };
        if let Some(want) = want
            && text.0 != want
        {
            text.0 = want;
        }
    }
    let blink_on = form.blink % 1.06 < 0.56;
    for (Caret(i), mut bg) in &mut carets {
        let on = *i == form.focus && blink_on && !busy;
        let want = if on { GOLD } else { Color::NONE };
        if bg.0 != want {
            bg.0 = want;
        }
    }
    for (FieldText(i, part), mut bg) in &mut selections {
        bg.0 = if *i == form.focus && !busy && matches!(*part, 1 | 2) {
            Color::srgba(0.20, 0.43, 0.50, 0.85)
        } else {
            Color::NONE
        };
    }
    for (FieldBox(i), mut border, mut bg) in &mut boxes {
        let focused = *i == form.focus && !busy;
        border.set_all(if focused { GOLD } else { GOLD_DIM });
        bg.0 = if focused { Color::srgba(0.04, 0.10, 0.12, 0.88) } else { FIELD_BG };
    }
    for (connect, exit, mut border, mut bg) in &mut buttons {
        let (hot, dim) = if connect.is_some() { (form.hover == Hover::Connect, busy) } else { (form.hover == Hover::Exit && exit.is_some(), false) };
        border.set_all(if hot && !dim { Color::srgb(1.0, 0.88, 0.55) } else if dim { GOLD_DIM } else if connect.is_some() { GOLD } else { GOLD_DIM });
        bg.0 = if connect.is_some() {
            if hot && !dim { Color::srgba(0.16, 0.34, 0.36, 0.95) } else { Color::srgba(0.10, 0.22, 0.24, if dim { 0.6 } else { 0.9 }) }
        } else if hot {
            Color::srgba(0.10, 0.22, 0.24, 0.85)
        } else {
            Color::srgba(0.02, 0.06, 0.08, 0.6)
        };
    }
    let now = time.elapsed_secs();
    let alpha = match form.status {
        Status::Done { at } => ((now - at) / FADE_OUT).clamp(0.0, 1.0),
        _ => 1.0 - (form.age / FADE_IN).clamp(0.0, 1.0),
    };
    for mut bg in &mut fade {
        bg.0 = Color::srgba(0.0, 0.0, 0.0, alpha * alpha * (3.0 - 2.0 * alpha));
    }
}

/// Keeps the caret in view in a field with more text than fits: the row shifts left.
fn scroll_fields(
    form: Res<LoginForm>,
    boxes: Query<(&FieldBox, &ComputedNode)>,
    befores: Query<(&FieldText, &ComputedNode)>,
    mut rows: Query<(&FieldRow, &mut Node)>,
) {
    for (FieldRow(i), mut node) in &mut rows {
        let Some((_, outer)) = boxes.iter().find(|(b, _)| b.0 == *i) else { continue };
        let inv = outer.inverse_scale_factor();
        let avail = (outer.size().x - outer.border().min_inset.x - outer.border().max_inset.x - outer.padding().min_inset.x - outer.padding().max_inset.x) * inv;
        let caret_x = befores.iter().filter(|(t, _)| t.0 == *i && t.1 < 2).map(|(_, n)| n.size().x * inv).sum::<f32>() + 6.0;
        let shift = if form.focus == *i { (avail - caret_x).min(0.0) } else { 0.0 };
        let want = px(shift);
        if node.left != want {
            node.left = want;
        }
    }
}
